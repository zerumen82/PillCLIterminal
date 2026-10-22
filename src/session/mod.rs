//! Una operación local activa; la consola nunca espera a la red.
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use tokio::sync::{mpsc as async_mpsc, watch};

use crate::{
    config::{Config, OllamaConfig},
    providers::ollama::{self, Message, Ollama, SelectedModel},
    tools::{Toolset, mcp::Offer},
};

#[derive(Clone, Debug)]
pub struct ListedModel {
    pub name: String,
    pub size: u64,
    pub loaded: bool,
}

#[derive(Debug)]
pub enum Action {
    Discover,
    Select(String),
    Chat(String),
    Task(String),
    Permissions,
    Grant { id: String, fingerprint: String },
    Revoke { id: String },
    Journal,
    Liberate,
    Cancel,
}

#[derive(Debug)]
pub enum Event {
    Models(Vec<ListedModel>),
    Selected(SelectedModel),
    Delta(String),
    Budget {
        used: usize,
        limit: u32,
    },
    Activity {
        role: String,
        text: String,
    },
    Reasoning {
        chars: usize,
        text: String,
    },
    /// El modelo recuperó mensajes de una ejecución anterior. No termina el trabajo.
    Restored {
        dropped: usize,
    },
    TaskStep {
        role: String,
        step: usize,
    },
    Notice {
        role: String,
        text: String,
    },
    Tool {
        role: String,
        call_id: String,
        name: String,
        arguments: serde_json::Value,
        result: serde_json::Value,
        journal: String,
    },
    Permissions(Vec<Offer>),
    Granted(String),
    Revoked(String),
    Journal(String),
    Liberated(String),
    Complete {
        elapsed_ms: u128,
        first_token_ms: Option<u128>,
        metrics: serde_json::Value,
        answer: Option<String>,
    },
    Failed(String),
    Cancelled,
}

pub struct Worker {
    jobs: Option<mpsc::SyncSender<(Action, u64)>>,
    cancel: watch::Sender<u64>,
    cancel_flag: Arc<AtomicBool>,
    liberating: Arc<AtomicBool>,
    events: async_mpsc::Receiver<Event>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    pub fn start(config: OllamaConfig) -> std::io::Result<Self> {
        Self::start_internal(config, None, None)
    }

    pub fn start_project(
        project: &Path,
        config: &Config,
        source: Option<&Path>,
        trust: &Path,
    ) -> std::io::Result<Self> {
        let tools = Toolset::new(project, config, source, trust).map_err(std::io::Error::other)?;
        Self::start_internal(
            config.ollama.clone(),
            Some(tools),
            Some(config.agents.clone()),
        )
    }

    fn start_internal(
        config: OllamaConfig,
        tools: Option<Toolset>,
        limits: Option<crate::config::AgentConfig>,
    ) -> std::io::Result<Self> {
        // Un lote de deltas acotado: contrapresión sin crecimiento ilimitado.
        let (events_tx, events) = async_mpsc::channel(config.event_buffer);
        let (jobs, jobs_rx) = mpsc::sync_channel(1);
        let (cancel, mut cancellation) = watch::channel(0u64);
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancellation_flag = cancel_flag.clone();
        let liberating = Arc::new(AtomicBool::new(false));
        let liberating_thread = liberating.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let thread = thread::Builder::new()
            .name("pill-ollama".into())
            .spawn(move || {
                let api = match Ollama::new(&config) {
                    Ok(api) => api,
                    Err(error) => {
                        let _ = events_tx.blocking_send(Event::Failed(error.to_string()));
                        return;
                    }
                };
                let mut selected = None;
                let mut messages = vec![Message::new("system", &config.system_prompt)];
                let mut owned = BTreeSet::new();
                let mut saved_transcript = tools.as_ref().and_then(load_model_transcript);
                let mut restore_open = true;
                if tools.is_some() {
                    let report = runtime.block_on(release(&api, tools.as_ref(), &owned));
                    let text = match report {
                        Ok(text) => text,
                        Err(error) => format!("liberación de arranque no completada: {error}"),
                    };
                    if events_tx
                        .blocking_send(Event::Notice {
                            role: "sistema".into(),
                            text,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                while let Ok((action, generation)) = jobs_rx.recv() {
                    let releasing = matches!(action, Action::Liberate);
                    if releasing {
                        liberating_thread.store(true, Ordering::SeqCst);
                    }
                    let _release_guard = ReleaseGuard {
                        flag: &liberating_thread,
                        armed: releasing,
                    };
                    let terminal = match runtime.block_on(events_tx.reserve()) {
                        Ok(permit) => permit,
                        Err(_) => break,
                    };
                    let already_cancelled = *cancellation.borrow_and_update() != generation;
                    if !already_cancelled {
                        cancellation_flag.store(false, Ordering::SeqCst);
                    }
                    let result = runtime.block_on(async {
                        if already_cancelled {
                            return None;
                        }
                        let result = execute(
                            action,
                            Execution {
                                api: &api,
                                selected: &selected,
                                messages: &messages,
                                events: &events_tx,
                                tools: tools.as_ref(),
                                limits: limits.as_ref(),
                                config: &config,
                                cancellation: &mut cancellation,
                                cancel: cancellation_flag.clone(),
                                owned: &owned,
                            },
                        )
                        .await;
                        if cancellation_flag.load(Ordering::SeqCst)
                            && !matches!(result, Ok(JobResult::Liberated(_)))
                        {
                            None
                        } else {
                            Some(result)
                        }
                    });
                    let event = match result {
                        None => Event::Cancelled,
                        Some(Err(error)) => Event::Failed(error.to_string()),
                        Some(Ok(JobResult::Models(models))) => Event::Models(models),
                        Some(Ok(JobResult::Permissions(proposals))) => {
                            Event::Permissions(proposals)
                        }
                        Some(Ok(JobResult::Granted(id))) => Event::Granted(id),
                        Some(Ok(JobResult::Revoked(id))) => Event::Revoked(id),
                        Some(Ok(JobResult::Journal(text))) => Event::Journal(text),
                        Some(Ok(JobResult::Liberated(text))) => Event::Liberated(text),
                        Some(Ok(JobResult::Engine(reply))) => {
                            messages = reply.transcript;
                            if let (Some(tools), Some(model)) = (tools.as_ref(), selected.as_ref())
                            {
                                store_model_transcript(
                                    tools,
                                    &model.model.name,
                                    &model.model.digest,
                                    &messages,
                                );
                            }
                            Event::Complete {
                                elapsed_ms: reply.elapsed_ms,
                                first_token_ms: reply.first_token_ms,
                                metrics: reply.metrics,
                                answer: Some(reply.content),
                            }
                        }
                        Some(Ok(JobResult::Selected(model))) => {
                            owned.insert(model.model.name.clone());
                            let name = model.model.name.clone();
                            let digest = model.model.digest.clone();
                            selected = Some(model.clone());
                            let first = std::mem::take(&mut restore_open);
                            let mut restored = None;
                            let mut persist = !first;
                            if first {
                                if let Some(saved) = saved_transcript.take() {
                                    let same = saved.model == name && saved.digest == digest;
                                    let dialogue = saved
                                        .messages
                                        .iter()
                                        .any(|message| message.role != "system");
                                    if same && dialogue {
                                        restored = Some(saved.dropped);
                                        messages = saved.messages;
                                        persist = true;
                                    } else if same {
                                        messages.truncate(1);
                                        persist = true;
                                    } else {
                                        messages.truncate(1);
                                    }
                                } else {
                                    messages.truncate(1);
                                    persist = true;
                                }
                            } else {
                                messages.truncate(1);
                            }
                            if persist && let Some(tools) = tools.as_ref() {
                                store_model_transcript(tools, &name, &digest, &messages);
                            }
                            if let Some(dropped) = restored {
                                let _ = events_tx.blocking_send(Event::Restored { dropped });
                            }
                            Event::Selected(model)
                        }
                        Some(Ok(JobResult::Chat { prompt, outcome })) => {
                            messages.push(Message::new("user", prompt));
                            messages.push(Message::new("assistant", outcome.content));
                            if let (Some(tools), Some(model)) = (tools.as_ref(), selected.as_ref())
                            {
                                store_model_transcript(
                                    tools,
                                    &model.model.name,
                                    &model.model.digest,
                                    &messages,
                                );
                            }
                            Event::Complete {
                                elapsed_ms: outcome.elapsed_ms,
                                first_token_ms: outcome.first_token_ms,
                                metrics: outcome.metrics,
                                answer: None,
                            }
                        }
                    };
                    if let Ok(used) =
                        ollama::reserved_context(&messages, &[], None, config.num_predict)
                    {
                        let _ = events_tx.blocking_send(Event::Budget {
                            used,
                            limit: config.num_ctx,
                        });
                    }
                    terminal.send(event);
                }
            })?;
        Ok(Self {
            jobs: Some(jobs),
            cancel,
            cancel_flag,
            liberating,
            events,
            thread: Some(thread),
        })
    }

    pub fn submit(&self, action: Action) -> Result<(), String> {
        if matches!(action, Action::Cancel) {
            self.cancel_flag.store(true, Ordering::SeqCst);
            self.cancel
                .send_modify(|generation| *generation = generation.wrapping_add(1));
            return Ok(());
        }
        if self.liberating.load(Ordering::SeqCst) {
            return Err("liberación en curso; espera a que termine antes de otra petición".into());
        }
        self.jobs
            .as_ref()
            .ok_or("sesión cerrada")?
            .try_send((action, *self.cancel.borrow()))
            .map_err(|e| format!("no se pudo iniciar la operación: {e}"))
    }

    pub fn next_event(&mut self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
        self.cancel
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        self.events.close();
        self.jobs.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum JobResult {
    Engine(crate::agents::Reply),
    Permissions(Vec<Offer>),
    Granted(String),
    Revoked(String),
    Journal(String),
    Liberated(String),
    Models(Vec<ListedModel>),
    Selected(SelectedModel),
    Chat {
        prompt: String,
        outcome: ollama::Outcome,
    },
}

struct ReleaseGuard<'a> {
    flag: &'a AtomicBool,
    armed: bool,
}

impl Drop for ReleaseGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.flag.store(false, Ordering::SeqCst);
        }
    }
}

struct Execution<'a> {
    api: &'a Ollama,
    selected: &'a Option<SelectedModel>,
    messages: &'a [Message],
    events: &'a async_mpsc::Sender<Event>,
    tools: Option<&'a Toolset>,
    limits: Option<&'a crate::config::AgentConfig>,
    config: &'a OllamaConfig,
    cancellation: &'a mut watch::Receiver<u64>,
    cancel: Arc<AtomicBool>,
    owned: &'a BTreeSet<String>,
}
async fn execute(action: Action, context: Execution<'_>) -> Result<JobResult, ollama::Error> {
    let Execution {
        api,
        selected,
        messages,
        events,
        tools,
        limits,
        config,
        cancellation,
        cancel,
        owned,
    } = context;
    match action {
        Action::Discover => {
            tokio::select! {
                biased;
                _ = cancellation.changed() => Err(ollama::Error::Protocol("consulta cancelada".into())),
                result = list_models(api) => result,
            }
        }
        Action::Select(name) => {
            tokio::select! {biased; _=cancellation.changed()=>Err(ollama::Error::Protocol("selección cancelada".into())), result=api.select(&name)=>Ok(JobResult::Selected(result?))}
        }
        Action::Permissions => Ok(JobResult::Permissions(match tools {
            Some(tools) => tools.offers().map_err(ollama::Error::Protocol)?,
            None => Vec::new(),
        })),
        Action::Grant { id, fingerprint } => {
            let tools =
                tools.ok_or_else(|| ollama::Error::Protocol("sesión sin propuestas".into()))?;
            tools
                .grant_offer(&id, &fingerprint)
                .map_err(ollama::Error::Protocol)?;
            Ok(JobResult::Granted(id))
        }
        Action::Revoke { id } => {
            let tools =
                tools.ok_or_else(|| ollama::Error::Protocol("sesión sin propuestas".into()))?;
            tools.revoke_offer(&id).map_err(ollama::Error::Protocol)?;
            Ok(JobResult::Revoked(id))
        }
        Action::Journal => {
            let tools =
                tools.ok_or_else(|| ollama::Error::Protocol("sesión sin herramientas".into()))?;
            Ok(JobResult::Journal(tools.files.journal_tail().map_err(
                |error| ollama::Error::Protocol(error.to_string()),
            )?))
        }
        Action::Liberate => Ok(JobResult::Liberated(release(api, tools, owned).await?)),
        Action::Task(prompt) => {
            let tools =
                tools.ok_or_else(|| ollama::Error::Protocol("sesión sin herramientas".into()))?;
            let selected = selected
                .as_ref()
                .ok_or_else(|| ollama::Error::Protocol("elige modelo".into()))?;
            let mut engine = crate::agents::Engine {
                api,
                selected,
                tools,
                limits: limits.unwrap(),
                config,
                events,
                cancel,
                cancellation,
                journal: tools.journal().map_err(ollama::Error::Protocol)?,
            };
            Ok(JobResult::Engine(engine.task(&prompt).await?))
        }
        Action::Chat(prompt) => {
            let model = selected
                .as_ref()
                .ok_or_else(|| ollama::Error::Protocol("elige primero un modelo".into()))?;
            if let (Some(tools), Some(limits)) = (tools, limits) {
                let mut engine = crate::agents::Engine {
                    api,
                    selected: model,
                    tools,
                    limits,
                    config,
                    events,
                    cancel,
                    cancellation,
                    journal: tools.journal().map_err(ollama::Error::Protocol)?,
                };
                return Ok(JobResult::Engine(engine.chat(&prompt, messages).await?));
            }
            let mut request = messages.to_owned();
            request.push(Message::new("user", &prompt));
            let used = ollama::reserved_context(&request, &[], None, config.num_predict)?;
            events
                .send(Event::Budget {
                    used,
                    limit: config.num_ctx,
                })
                .await
                .map_err(|_| ollama::Error::Protocol("consola cerrada".into()))?;
            let request_future = api.chat(model, &request, |update| async {
                let event = match update {
                    ollama::StreamUpdate::Text(text) => Event::Delta(text),
                    ollama::StreamUpdate::Thinking { chars, text } => {
                        Event::Reasoning { chars, text }
                    }
                };
                events
                    .send(event)
                    .await
                    .map_err(|_| ollama::Error::Protocol("consola cerrada".into()))
            });
            let outcome = tokio::select! {biased; _=cancellation.changed()=>return Err(ollama::Error::Protocol("inferencia cancelada".into())), result=request_future=>result?};
            Ok(JobResult::Chat { prompt, outcome })
        }
        Action::Cancel => Err(ollama::Error::Protocol(
            "cancelación fuera de operación".into(),
        )),
    }
}

async fn list_models(api: &Ollama) -> Result<JobResult, ollama::Error> {
    let models = api.models().await?;
    let loaded = match api.running().await {
        Ok(value) => parse_running(&value)
            .map(|items| {
                items
                    .into_iter()
                    .map(|model| model.name)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default(),
        Err(_) => BTreeSet::new(),
    };
    Ok(JobResult::Models(
        models
            .into_iter()
            .map(|model| ListedModel {
                loaded: loaded.contains(&model.name),
                name: model.name,
                size: model.size,
            })
            .collect(),
    ))
}

fn parse_running(
    value: &serde_json::Value,
) -> Result<Vec<crate::gpu::RunningModel>, ollama::Error> {
    let models = value
        .get("models")
        .and_then(|item| item.as_array())
        .ok_or_else(|| ollama::Error::Protocol("/api/ps no trae la lista de modelos".into()))?;
    let mut running = Vec::new();
    for model in models {
        let name = model
            .get("name")
            .and_then(|item| item.as_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                ollama::Error::Protocol("/api/ps contiene un modelo sin nombre".into())
            })?;
        let digest = model
            .get("digest")
            .and_then(|item| item.as_str())
            .unwrap_or("")
            .to_owned();
        running.push(crate::gpu::RunningModel {
            name: name.to_owned(),
            digest,
        });
    }
    Ok(running)
}

async fn release(
    api: &Ollama,
    tools: Option<&Toolset>,
    owned: &BTreeSet<String>,
) -> Result<String, ollama::Error> {
    let Some(tools) = tools else {
        return Ok("sesión sin herramientas: no se descarga ningún modelo".into());
    };
    let exclusive = tools.exclusive_granted().map_err(ollama::Error::Protocol)?;
    let running = parse_running(&api.running().await?)?;
    let plan = crate::gpu::plan(exclusive, owned, &running);
    let mut lines = vec![plan.reason];
    if !plan.unload.is_empty() {
        for name in &plan.unload {
            api.unload(name).await?;
        }
        let after = parse_running(&api.running().await?)?;
        let still: Vec<_> = plan
            .unload
            .iter()
            .filter(|name| after.iter().any(|model| model.name == **name))
            .cloned()
            .collect::<Vec<_>>();
        if !still.is_empty() {
            return Err(ollama::Error::Protocol(format!(
                "la descarga no se confirmó; siguen en memoria: {}",
                still.join(", ")
            )));
        }
        lines.push(format!("descargados: {}", plan.unload.join(", ")));
    }
    if !plan.conserved.is_empty() {
        lines.push(format!("conservados: {}", plan.conserved.join(", ")));
    }
    lines.push(crate::gpu::nvidia_line(tools.nvidia_path()));
    Ok(lines.join("\n"))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ModelTranscript {
    model: String,
    digest: String,
    #[serde(default)]
    dropped: usize,
    messages: Vec<Message>,
}

fn load_model_transcript(tools: &Toolset) -> Option<ModelTranscript> {
    let bytes = tools.files.read_private("model.json").ok().flatten()?;
    serde_json::from_slice(&bytes).ok()
}

fn store_model_transcript(tools: &Toolset, model: &str, digest: &str, messages: &[Message]) {
    let Some(bytes) = fitted_transcript(model, digest, messages, tools.journal_byte_limit()) else {
        return;
    };
    let _ = tools.files.write_private("model.json", &bytes);
}

/// Copia para disco. El historial en memoria no se recorta: si no cabe, se omiten
/// los mensajes antiguos que no son el sistema. None si ni eso cabe.
fn fitted_transcript(
    model: &str,
    digest: &str,
    messages: &[Message],
    limit: usize,
) -> Option<Vec<u8>> {
    let mut kept = messages.to_vec();
    let mut dropped = 0usize;
    loop {
        let body = ModelTranscript {
            model: model.to_owned(),
            digest: digest.to_owned(),
            dropped,
            messages: kept.clone(),
        };
        let bytes = serde_json::to_vec(&body).ok()?;
        if bytes.len() <= limit {
            return Some(bytes);
        }
        if kept.len() <= 1 {
            return None;
        }
        let index = usize::from(kept[0].role == "system");
        kept.remove(index);
        dropped += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_transcript_drops_the_oldest_dialogue_until_it_fits() {
        let mut messages = vec![Message::new("system", "sistema")];
        for index in 0..20 {
            messages.push(Message::new(
                "user",
                format!("pregunta {index} {}", "x".repeat(80)),
            ));
            messages.push(Message::new("assistant", format!("respuesta {index}")));
        }
        let bytes = fitted_transcript("modelo", "digest", &messages, 900).unwrap();
        assert!(bytes.len() <= 900);
        let saved: ModelTranscript = serde_json::from_slice(&bytes).unwrap();
        assert!(saved.dropped > 0);
        assert_eq!(saved.messages[0].role, "system");
        assert!(
            !saved
                .messages
                .iter()
                .any(|message| message.content.contains("pregunta 0"))
        );
        assert!(
            saved
                .messages
                .iter()
                .any(|message| message.role == "assistant")
        );
        assert!(fitted_transcript("modelo", "digest", &messages, 8).is_none());
    }
}
