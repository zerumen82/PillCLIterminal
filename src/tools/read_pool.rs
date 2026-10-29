//! Trabajadores nativos reutilizados; no crean runtimes ni ejecutan efectos.
use super::Role;
use crate::{
    files::{FileError, Files},
    providers::ollama::ToolCall,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Instant,
};

type Observations = Arc<Mutex<BTreeMap<String, String>>>;
#[derive(Clone)]
pub(super) struct Context {
    pub files: Arc<Files>,
    pub observations: Observations,
    pub text_observations: Observations,
}
struct Job {
    role: Role,
    call: ToolCall,
    cancel: Arc<AtomicBool>,
    origin: Instant,
    reply: mpsc::SyncSender<(Value, Value)>,
}
pub(super) struct ReadPool {
    send: Option<mpsc::SyncSender<Job>>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl ReadPool {
    pub fn new(count: usize, context: Context) -> Result<Self, String> {
        if !(1..=8).contains(&count) {
            return Err("concurrencia de lecturas inválida".into());
        }
        let (send, receive) = mpsc::sync_channel::<Job>(count);
        let receive = Arc::new(Mutex::new(receive));
        let mut pool = Self {
            send: Some(send),
            workers: Vec::new(),
        };
        for index in 0..count {
            let receiver = receive.clone();
            let context = context.clone();
            pool.workers.push(thread::Builder::new().name(format!("pill-read-{index}")).spawn(move||loop {
                let received=receiver.lock().unwrap_or_else(|e|e.into_inner()).recv();
                let Ok(job)=received else {break;};
                let started=job.origin.elapsed().as_nanos();
                let value=if job.cancel.load(Ordering::SeqCst) {json!({"status":"cancelled","error":"lectura cancelada antes de iniciar"})}
                    else if !job.role.permits(&job.call.function.name) {json!({"status":"failed","error":"lectura no concedida a este rol"})}
                    else {context.execute(&job.call)};
                let execution=json!({"thread":format!("{:?}",thread::current().id()),"started_ns":started,"finished_ns":job.origin.elapsed().as_nanos()});
                let _=job.reply.send((value,execution));
            }).map_err(|e|e.to_string())?);
        }
        Ok(pool)
    }
    pub fn execute(
        &self,
        role: Role,
        calls: &[ToolCall],
        cancel: Arc<AtomicBool>,
    ) -> Vec<(Value, Value)> {
        let origin = Instant::now();
        let mut replies = Vec::new();
        for call in calls {
            let (reply, receive) = mpsc::sync_channel(1);
            let job = Job {
                role,
                call: call.clone(),
                cancel: cancel.clone(),
                origin,
                reply,
            };
            if self
                .send
                .as_ref()
                .is_none_or(|send| send.send(job).is_err())
            {
                return calls
                    .iter()
                    .map(|_| {
                        (
                            json!({"status":"failed","error":"trabajadores de lectura cerrados"}),
                            Value::Null,
                        )
                    })
                    .collect();
            }
            replies.push(receive);
        }
        replies
            .into_iter()
            .map(|receive| {
                receive.recv().unwrap_or_else(|_| {
                    (
                        json!({"status":"failed","error":"trabajador de lectura interrumpido"}),
                        Value::Null,
                    )
                })
            })
            .collect()
    }
}
impl Drop for ReadPool {
    fn drop(&mut self) {
        self.send.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Context {
    fn execute(&self, call: &ToolCall) -> Value {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct PathArgs {
            path: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct SearchArgs {
            path: String,
            needle: String,
        }
        let args = call.function.arguments.clone();
        let result = match call.function.name.as_str() {
            "read_file" | "list_files" | "inspect_path" => serde_json::from_value::<PathArgs>(args)
                .map_err(|e| e.to_string())
                .and_then(|args| {
                    let value = match call.function.name.as_str() {
                        "read_file" => self.files.read(&args.path),
                        "list_files" => self.files.list(&args.path),
                        _ => match self.files.inspect_operation(&args.path) {
                            Err(FileError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                                Ok(json!({"status":"verified","path":args.path,"absent":true}))
                            }
                            value => value,
                        },
                    };
                    value.map_err(|e| e.data().to_string())
                }),
            "search_files" => serde_json::from_value::<SearchArgs>(args)
                .map_err(|e| e.to_string())
                .and_then(|a| {
                    self.files
                        .search(&a.path, &a.needle)
                        .map_err(|e| e.data().to_string())
                }),
            _ => Err("solo lecturas nativas en trabajadores".into()),
        };
        let result = result.unwrap_or_else(|e| {
            serde_json::from_str(&e).unwrap_or_else(|_| json!({"status":"failed","error":e}))
        });
        if matches!(call.function.name.as_str(), "read_file" | "inspect_path")
            && result["status"] == "verified"
            && let (Some(path), Some(sha)) = (result["path"].as_str(), result["sha256"].as_str())
        {
            self.observations
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(path.into(), sha.into());
            if call.function.name == "read_file" {
                self.text_observations
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(path.into(), sha.into());
            }
        }
        result
    }
}
