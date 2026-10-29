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
    pub loaded: Option<bool>,
}
impl ListedModel {
    pub fn memory_label(&self) -> &'static str {
        match self.loaded {
            Some(true) => "en memoria",
            Some(false) => "no cargado",
            None => "memoria desconocida",
        }
    }
}

#[derive(Debug)]
pub enum Action {
    Discover,
    Select(String),
    Chat(String),
    Ask(String),
    Task(String),
    Recovery,
    AcknowledgeRecovery {
        fingerprint: String,
    },
    Changes,
    Undo {
        path: String,
        expected_sha256: String,
        backup_path: String,
        backup_sha256: String,
    },
    Status,
    Profiles,
    SelectProfile(String),
    Archive,
    ConfirmArchive {
        fingerprint: String,
    },
    Permissions,
    Grant {
        id: String,
        fingerprint: String,
    },
    Revoke {
        id: String,
    },
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
    Recovery {
        report: serde_json::Value,
    },
    RecoveryReviewed {
        report: serde_json::Value,
    },
    Changes {
        items: Vec<serde_json::Value>,
    },
    UndoComplete {
        result: serde_json::Value,
    },
    Status {
        report: serde_json::Value,
    },
    Profiles(Vec<serde_json::Value>),
    ProfileSelected {
        report: serde_json::Value,
    },
    Archive {
        report: serde_json::Value,
    },
    ArchiveComplete {
        report: serde_json::Value,
    },
    StorageWarning(String),
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
        Self::start_internal(config, None, None, std::collections::BTreeMap::new())
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
            config.profiles.clone(),
        )
    }

    fn start_internal(
        mut config: OllamaConfig,
        tools: Option<Toolset>,
        limits: Option<crate::config::AgentConfig>,
        profiles: std::collections::BTreeMap<String, crate::config::OllamaProfile>,
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
                let mut api = match Ollama::new(&config) {
                    Ok(api) => api,
                    Err(error) => {
                        let _ = events_tx.blocking_send(Event::Failed(error.to_string()));
                        return;
                    }
                };
                let mut selected = None;
                let mut messages = vec![Message::new("system", &config.system_prompt)];
                let mut owned = BTreeSet::new();
                let mut saved_transcript = match tools
                    .as_ref()
                    .map(|tools| load_model_transcript(tools, &config.system_prompt))
                {
                    Some(Ok((saved, warning))) => {
                        if let Some(warning) = warning {
                            let _ = events_tx.blocking_send(Event::StorageWarning(warning));
                        }
                        saved
                    }
                    Some(Err(error)) => {
                        let _ = events_tx.blocking_send(Event::StorageWarning(format!(
                            "conversación no recuperada: {error}"
                        )));
                        None
                    }
                    None => None,
                };
                let mut restore_open = true;
                if tools.is_some() {
                    let report = runtime.block_on(release(&api, tools.as_ref(), &owned));
                    let text = match report {
                        Ok(text) => text,
                        Err(error) => format!("liberación de arranque no completada: {error}"),
                    };
                    let text = if let Some(tools) = &tools {
                        format!("{text}\n{}", tools.recovery_report())
                    } else {
                        text
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
                                profiles: &profiles,
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
                        Some(Ok(JobResult::Recovery(report))) => Event::Recovery { report },
                        Some(Ok(JobResult::RecoveryReviewed(report))) => {
                            Event::RecoveryReviewed { report }
                        }
                        Some(Ok(JobResult::Changes(items))) => Event::Changes { items },
                        Some(Ok(JobResult::UndoComplete(result))) => Event::UndoComplete { result },
                        Some(Ok(JobResult::Status(report))) => Event::Status { report },
                        Some(Ok(JobResult::Profiles(items))) => Event::Profiles(items),
                        Some(Ok(JobResult::ProfileSelected(report))) => {
                            match profiles
                                .get(report["name"].as_str().unwrap_or(""))
                                .ok_or_else(|| "perfil desaparecido".to_string())
                                .and_then(|profile| profile.apply(&config))
                            {
                                Ok(candidate) => match Ollama::new(&candidate) {
                                    Ok(candidate_api) => {
                                        config = candidate;
                                        api = candidate_api;
                                        Event::ProfileSelected { report }
                                    }
                                    Err(error) => Event::Failed(error.to_string()),
                                },
                                Err(error) => Event::Failed(error),
                            }
                        }
                        Some(Ok(JobResult::Archive(report))) => Event::Archive { report },
                        Some(Ok(JobResult::ArchiveComplete(report))) => {
                            Event::ArchiveComplete { report }
                        }
                        Some(Ok(JobResult::Engine(reply))) => {
                            messages = reply.transcript;
                            if let (Some(tools), Some(model)) = (tools.as_ref(), selected.as_ref())
                            {
                                warn_storage(
                                    &events_tx,
                                    store_model_transcript(
                                        tools,
                                        &model.model.name,
                                        &model.model.digest,
                                        &messages,
                                    ),
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
                                warn_storage(
                                    &events_tx,
                                    store_model_transcript(tools, &name, &digest, &messages),
                                );
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
                                warn_storage(
                                    &events_tx,
                                    store_model_transcript(
                                        tools,
                                        &model.model.name,
                                        &model.model.digest,
                                        &messages,
                                    ),
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
    Recovery(serde_json::Value),
    RecoveryReviewed(serde_json::Value),
    Changes(Vec<serde_json::Value>),
    UndoComplete(serde_json::Value),
    Status(serde_json::Value),
    Profiles(Vec<serde_json::Value>),
    ProfileSelected(serde_json::Value),
    Archive(serde_json::Value),
    ArchiveComplete(serde_json::Value),
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
    profiles: &'a std::collections::BTreeMap<String, crate::config::OllamaProfile>,
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
        profiles,
    } = context;
    let read_only = matches!(&action, Action::Ask(_));
    match action {
        Action::Status => Ok(JobResult::Status(
            status_report(api, tools, selected, owned, config).await,
        )),
        Action::Profiles => Ok(JobResult::Profiles(
            profiles
                .iter()
                .map(|(name, p)| serde_json::json!({"name":name,"profile":p}))
                .collect(),
        )),
        Action::SelectProfile(name) => {
            let candidate = profiles
                .get(&name)
                .ok_or_else(|| ollama::Error::Protocol("perfil no configurado".into()))?
                .apply(config)
                .map_err(ollama::Error::Protocol)?;
            let candidate_api = Ollama::new(&candidate)?;
            if let Some(model) = selected {
                let fresh = candidate_api.select(&model.model.name).await?;
                if fresh.model.digest != model.model.digest {
                    return Err(ollama::Error::Protocol(
                        "el modelo cambió; selección explícita necesaria".into(),
                    ));
                }
            }
            let used = ollama::reserved_context(messages, &[], None, candidate.num_predict)?;
            if used > candidate.num_ctx as usize {
                return Err(ollama::Error::Context {
                    used,
                    limit: candidate.num_ctx,
                });
            }
            let mut report =
                status_report(&candidate_api, tools, selected, owned, &candidate).await;
            report["name"] = serde_json::json!(name);
            report["profile"] =
                serde_json::to_value(profiles.get(&name)).map_err(ollama::Error::Json)?;
            Ok(JobResult::ProfileSelected(report))
        }
        Action::Recovery => Ok(JobResult::Recovery(
            tools
                .ok_or_else(|| ollama::Error::Protocol("sesión sin herramientas".into()))?
                .recovery_view()
                .map_err(ollama::Error::Protocol)?,
        )),
        Action::AcknowledgeRecovery { fingerprint } => Ok(JobResult::RecoveryReviewed(
            tools
                .ok_or_else(|| ollama::Error::Protocol("sesión sin herramientas".into()))?
                .acknowledge_recovery(&fingerprint)
                .map_err(ollama::Error::Protocol)?,
        )),
        Action::Changes => Ok(JobResult::Changes(
            tools
                .ok_or_else(|| ollama::Error::Protocol("sesión sin herramientas".into()))?
                .changes(),
        )),
        Action::Undo {
            path,
            expected_sha256,
            backup_path,
            backup_sha256,
        } => Ok(JobResult::UndoComplete(
            tools
                .ok_or_else(|| ollama::Error::Protocol("sesión sin herramientas".into()))?
                .undo(&path, &expected_sha256, &backup_path, &backup_sha256)
                .map_err(ollama::Error::Protocol)?,
        )),
        Action::Archive => Ok(JobResult::Archive(
            tools
                .ok_or_else(|| ollama::Error::Protocol("sesión sin herramientas".into()))?
                .archive_offers()
                .map_err(ollama::Error::Protocol)?,
        )),
        Action::ConfirmArchive { fingerprint } => Ok(JobResult::ArchiveComplete(
            tools
                .ok_or_else(|| ollama::Error::Protocol("sesión sin herramientas".into()))?
                .confirm_archive(&fingerprint)
                .map_err(ollama::Error::Protocol)?,
        )),
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
        Action::Ask(prompt) | Action::Chat(prompt) => {
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
                return Ok(JobResult::Engine(if read_only {
                    engine.ask(&prompt, messages).await?
                } else {
                    engine.chat(&prompt, messages).await?
                }));
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

async fn status_report(
    api: &Ollama,
    tools: Option<&Toolset>,
    selected: &Option<SelectedModel>,
    owned: &BTreeSet<String>,
    config: &OllamaConfig,
) -> serde_json::Value {
    use serde_json::{Value, json};
    let (running, error) = match api.running().await.and_then(|value| {
        parse_running(&value)?;
        Ok(value)
    }) {
        Ok(value) => (value, None),
        Err(error) => (Value::Null, Some(error.to_string())),
    };
    let current = selected.as_ref().and_then(|s| {
        running["models"]
            .as_array()?
            .iter()
            .find(|m| m["name"] == s.model.name && m["digest"] == s.model.digest)
    });
    let retention = tools.map(|t| t.retention_status()).transpose();
    let (quotas, storage_error) = match retention {
        Ok(value) => (value, None),
        Err(error) => (None, Some(error)),
    };
    json!({"url":config.url,"model":{"selected":selected.as_ref().map(|m|&m.model.name),"digest":selected.as_ref().map(|m|&m.model.digest),"loaded":if error.is_some(){None}else{Some(current.is_some())},"owned":selected.as_ref().is_some_and(|m|owned.contains(&m.model.name)),"size_vram":current.and_then(|m|m.get("size_vram"))},"context":{"requested":config.num_ctx,"observed":current.and_then(|m|m.get("context_length"))},"num_ctx":config.num_ctx,"num_predict":config.num_predict,"think":config.think,"temperature":config.temperature,"retention":quotas,"connection_error":error,"storage_error":storage_error})
}

async fn list_models(api: &Ollama) -> Result<JobResult, ollama::Error> {
    let models = api.models().await?;
    let loaded = api
        .running()
        .await
        .and_then(|value| parse_running(&value))
        .ok()
        .map(|items| {
            items
                .into_iter()
                .map(|model| model.name)
                .collect::<BTreeSet<_>>()
        });
    Ok(JobResult::Models(
        models
            .into_iter()
            .map(|model| ListedModel {
                loaded: loaded.as_ref().map(|names| names.contains(&model.name)),
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
            .filter(|digest| !digest.is_empty())
            .ok_or_else(|| ollama::Error::Protocol("/api/ps contiene un modelo sin digest".into()))?
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
#[serde(deny_unknown_fields)]
struct ModelTranscript {
    #[serde(default = "legacy_transcript_version")]
    schema_version: u32,
    model: String,
    digest: String,
    dropped: usize,
    messages: Vec<Message>,
}

fn legacy_transcript_version() -> u32 {
    1
}

fn complete_turns(messages: &[Message]) -> Result<Vec<Vec<Message>>, String> {
    if messages
        .first()
        .is_none_or(|message| message.role != "system")
    {
        return Err("historial sin sistema inicial".into());
    }
    let mut turns = Vec::new();
    let mut turn = Vec::new();
    let mut pending = std::collections::VecDeque::new();
    for message in &messages[1..] {
        match message.role.as_str() {
            "user" => {
                if !message.tool_calls.is_empty()
                    || message.tool_name.is_some()
                    || !message.thinking.is_empty()
                {
                    return Err("metadatos de herramienta en petición".into());
                }
                if !pending.is_empty() {
                    return Err("turno con resultados de herramientas pendientes".into());
                }
                if !turn.is_empty() {
                    if turn.last().is_none_or(|message: &Message| {
                        message.role != "assistant" || !message.tool_calls.is_empty()
                    }) {
                        return Err("turno incompleto".into());
                    }
                    turns.push(std::mem::take(&mut turn));
                }
            }
            "assistant" => {
                if message.tool_name.is_some() {
                    return Err("nombre de herramienta en respuesta assistant".into());
                }
                if turn.is_empty() || !pending.is_empty() {
                    return Err("respuesta sin petición o con resultados pendientes".into());
                }
                pending.extend(
                    message
                        .tool_calls
                        .iter()
                        .map(|call| call.function.name.clone()),
                );
            }
            "tool" => {
                if !message.tool_calls.is_empty() || !message.thinking.is_empty() {
                    return Err("metadatos de respuesta en resultado tool".into());
                }
                if pending.pop_front().as_deref() != message.tool_name.as_deref()
                    || message.tool_name.is_none()
                {
                    return Err("resultado de herramienta sin llamada correspondiente".into());
                }
            }
            _ => return Err("rol inesperado en historial guardado".into()),
        }
        turn.push(message.clone());
    }
    if !pending.is_empty()
        || (!turn.is_empty()
            && turn.last().is_none_or(|message| {
                message.role != "assistant" || !message.tool_calls.is_empty()
            }))
    {
        return Err("último turno incompleto".into());
    }
    if !turn.is_empty() {
        turns.push(turn);
    }
    Ok(turns)
}

pub(crate) fn valid_transcript(bytes: &[u8]) -> bool {
    serde_json::from_slice::<ModelTranscript>(bytes).is_ok_and(|saved| {
        matches!(saved.schema_version, 1 | 2) && complete_turns(&saved.messages).is_ok()
    })
}

fn load_model_transcript(
    tools: &Toolset,
    system: &str,
) -> Result<(Option<ModelTranscript>, Option<String>), String> {
    let read = tools
        .files
        .read_private_checked_with("model.json", valid_transcript)
        .map_err(|e| e.to_string())?;
    let Some(bytes) = read.bytes else {
        return Ok((None, read.warning));
    };
    let mut saved: ModelTranscript = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if !matches!(saved.schema_version, 1 | 2) {
        return Err("versión de conversación no admitida".into());
    }
    complete_turns(&saved.messages)?;
    saved.messages[0] = Message::new("system", system);
    saved.schema_version = 2;
    Ok((Some(saved), read.warning))
}

fn warn_storage(events: &async_mpsc::Sender<Event>, result: Result<(), String>) {
    if let Err(error) = result {
        let _ = events.blocking_send(Event::StorageWarning(format!(
            "conversación no guardada: {error}"
        )));
    }
}

fn store_model_transcript(
    tools: &Toolset,
    model: &str,
    digest: &str,
    messages: &[Message],
) -> Result<(), String> {
    let bytes = fitted_transcript(model, digest, messages, tools.journal_byte_limit())?;
    tools
        .files
        .write_private("model.json", &bytes)
        .map_err(|e| e.to_string())
}

/// La copia persistida conserva turnos completos y no modifica el historial activo.
fn fitted_transcript(
    model: &str,
    digest: &str,
    messages: &[Message],
    limit: usize,
) -> Result<Vec<u8>, String> {
    let turns = complete_turns(messages)?;
    let system = messages[0].clone();
    for first in 0..=turns.len() {
        let mut kept = vec![system.clone()];
        kept.extend(turns[first..].iter().flatten().cloned());
        let dropped = turns[..first].iter().map(Vec::len).sum();
        let body = ModelTranscript {
            schema_version: 2,
            model: model.into(),
            digest: digest.into(),
            dropped,
            messages: kept,
        };
        let bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        if bytes.len() <= limit {
            return Ok(bytes);
        }
    }
    Err("ni el sistema cabe en el límite de persistencia".into())
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
        assert!(fitted_transcript("modelo", "digest", &messages, 8).is_err());
    }
    #[test]
    fn persisted_turns_never_leave_tool_results_without_their_calls() {
        use crate::providers::ollama::{ToolCall, ToolFunction};
        let mut assistant = Message::new("assistant", "");
        assistant.tool_calls = vec![ToolCall {
            function: ToolFunction {
                name: "read_file".into(),
                arguments: serde_json::json!({"path":"a"}),
            },
        }];
        let mut result = Message::new("tool", "{\"status\":\"verified\"}");
        result.tool_name = Some("read_file".into());
        let messages = vec![
            Message::new("system", "sistema"),
            Message::new("user", "x".repeat(1500)),
            assistant,
            result,
            Message::new("assistant", "leído"),
            Message::new("user", "pregunta corta"),
            Message::new("assistant", "respuesta corta"),
        ];
        let bytes = fitted_transcript("m", "d", &messages, 700).unwrap();
        let saved: ModelTranscript = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(saved.schema_version, 2);
        assert_eq!(saved.messages.len(), 3);
        assert_eq!(saved.messages[1].content, "pregunta corta");
        assert_eq!(saved.dropped, 4);
        assert!(complete_turns(&saved.messages).is_ok());
        let mut orphan = messages.clone();
        orphan.remove(2);
        assert!(fitted_transcript("m", "d", &orphan, 10_000).is_err());
    }

    #[test]
    fn semantic_history_corruption_recovers_backup_and_replaces_saved_system() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let config: Config =
            serde_json::from_str(include_str!("../../config/defaults.json")).unwrap();
        let tools = Toolset::new(
            &root,
            &config,
            None,
            &temp.path().join("private/trust.json"),
        )
        .unwrap();
        let messages = vec![
            Message::new("system", "sistema antiguo"),
            Message::new("user", "hola"),
            Message::new("assistant", "respuesta"),
        ];
        store_model_transcript(&tools, "m", "d", &messages).unwrap();
        tools.files.write_private("model.json",br#"{"schema_version":2,"model":"m","digest":"d","dropped":0,"messages":[{"role":"system","content":"viejo"},{"role":"tool","content":"orphan","tool_name":"read_file"}]}"#).unwrap();
        let (saved, warning) = load_model_transcript(&tools, "sistema vigente").unwrap();
        let saved = saved.unwrap();
        assert!(warning.is_some());
        assert_eq!(saved.messages[0].content, "sistema vigente");
        assert_eq!(saved.messages[1].content, "hola");
        assert_eq!(saved.schema_version, 2);
    }

    #[test]
    fn storage_warning_is_an_explicit_nonterminal_event() {
        let (tx, mut rx) = async_mpsc::channel(2);
        warn_storage(&tx, Err("disco lleno".into()));
        assert!(
            matches!(rx.try_recv().unwrap(),Event::StorageWarning(text) if text.contains("disco lleno"))
        );
    }
}
