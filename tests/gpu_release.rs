#![cfg(windows)]

use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use pill::{
    config::Config,
    gpu::exclusive_fingerprint,
    session::{Action, Event, Worker},
    tools::Toolset,
};
use serde_json::{Value, json};

struct State {
    loaded: Mutex<BTreeSet<String>>,
    generates: Mutex<Vec<Value>>,
    ps_count: AtomicUsize,
    hold_at: AtomicUsize,
    released: AtomicBool,
    honor: AtomicBool,
}

struct Probe {
    port: u16,
    state: Arc<State>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Probe {
    fn start(loaded: &[&str], honor: bool, hold_at: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(State {
            loaded: Mutex::new(loaded.iter().map(|name| (*name).to_owned()).collect()),
            generates: Mutex::new(Vec::new()),
            ps_count: AtomicUsize::new(0),
            hold_at: AtomicUsize::new(hold_at),
            released: AtomicBool::new(false),
            honor: AtomicBool::new(honor),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let thread_state = state.clone();
        let thread_stop = stop.clone();
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                let Ok((stream, _)) = listener.accept() else {
                    break;
                };
                if thread_stop.load(Ordering::SeqCst) {
                    break;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                handle(stream, &thread_state);
            }
        });
        Self {
            port,
            state,
            stop,
            thread: Some(thread),
        }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle(mut stream: TcpStream, state: &State) {
    let Some((method, path, body)) = read_request(&mut stream) else {
        return;
    };
    let path = path.split('?').next().unwrap_or("");
    let payload = if path.ends_with("/api/ps") && method == "GET" {
        let count = state.ps_count.fetch_add(1, Ordering::SeqCst) + 1;
        let hold_at = state.hold_at.load(Ordering::SeqCst);
        if hold_at > 0 && count >= hold_at {
            let started = Instant::now();
            while !state.released.load(Ordering::SeqCst)
                && started.elapsed() < Duration::from_secs(20)
            {
                thread::sleep(Duration::from_millis(20));
            }
        }
        let models: Vec<_> = state
            .loaded
            .lock()
            .unwrap()
            .iter()
            .map(|name| json!({"name": name, "digest": format!("d-{name}"), "size": 1}))
            .collect();
        json!({"models": models})
    } else if path.ends_with("/api/generate") && method == "POST" {
        let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
        state.generates.lock().unwrap().push(value.clone());
        let model = value.get("model").and_then(Value::as_str).unwrap_or("");
        let unload = value.get("keep_alive").and_then(Value::as_i64) == Some(0)
            && value.get("stream").and_then(Value::as_bool) == Some(false);
        if unload && state.honor.load(Ordering::SeqCst) {
            state.loaded.lock().unwrap().remove(model);
        }
        json!({"done": true})
    } else if path.ends_with("/api/tags") && method == "GET" {
        json!({"models":[{"name":"propio","digest":"abc","size":1}]})
    } else if path.ends_with("/api/show") && method == "POST" {
        json!({
            "capabilities": ["completion", "tools"],
            "model_info": {"general.architecture": "qwen2", "qwen2.context_length": 32768}
        })
    } else {
        json!({"error": format!("ruta no usada por la prueba: {method} {path}")})
    };
    let bytes = payload.to_string().into_bytes();
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&bytes);
}

fn read_request(stream: &mut TcpStream) -> Option<(String, String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut temp = [0u8; 2048];
    let header_end = loop {
        let count = stream.read(&mut temp).ok()?;
        if count == 0 {
            return None;
        }
        buf.extend_from_slice(&temp[..count]);
        if let Some(position) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if buf.len() > 65_536 {
            return None;
        }
    };
    let header = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut length = 0usize;
    for line in header.lines() {
        if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = rest.trim().parse().unwrap_or(0);
        }
    }
    while buf.len() < header_end + length {
        let count = stream.read(&mut temp).ok()?;
        if count == 0 {
            break;
        }
        buf.extend_from_slice(&temp[..count]);
    }
    let body = buf
        .get(header_end..header_end + length)
        .unwrap_or_default()
        .to_vec();
    let request = header.lines().next().unwrap_or("");
    let mut parts = request.split_whitespace();
    Some((
        parts.next().unwrap_or("").to_owned(),
        parts.next().unwrap_or("").to_owned(),
        body,
    ))
}

struct Session {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    trust: PathBuf,
    config: Config,
}

fn session(url: &str, exclusive: bool) -> Session {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("proyecto");
    std::fs::create_dir(&root).unwrap();
    let mut config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    config.ollama.url = url.into();
    config.gpu.exclusive = exclusive;
    config.gpu.nvidia_smi = Some(PathBuf::from(r"D:\no-existe\nvidia-smi.exe"));
    let trust = temporary.path().join("privado").join("trust.json");
    Session {
        _temporary: temporary,
        root,
        trust,
        config,
    }
}

fn wait(worker: &mut Worker) -> Event {
    let start = Instant::now();
    loop {
        if let Some(event) = worker.next_event() {
            if matches!(
                event,
                Event::Budget { .. }
                    | Event::Activity { .. }
                    | Event::Reasoning { .. }
                    | Event::Restored { .. }
                    | Event::TaskStep { .. }
            ) {
                continue;
            }
            return event;
        }
        if start.elapsed() > Duration::from_secs(15) {
            panic!("la sesión no produjo un evento");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn grant_exclusive(session: &Session) {
    let tools = Toolset::new(&session.root, &session.config, None, &session.trust).unwrap();
    let fingerprint = exclusive_fingerprint(&session.config.ollama.url);
    tools.grant_offer("exclusive", &fingerprint).unwrap();
    assert!(tools.exclusive_granted().unwrap());
}

#[test]
fn shared_mode_never_calls_generate_even_for_a_selected_model() {
    let probe = Probe::start(&["propio", "other"], true, 0);
    let session = session(&probe.url(), false);
    let mut worker =
        Worker::start_project(&session.root, &session.config, None, &session.trust).unwrap();
    let startup = wait(&mut worker);
    match &startup {
        Event::Notice { text, .. } => assert!(text.contains("no se descarga"), "{text}"),
        other => panic!("arranque inesperado: {other:?}"),
    }
    assert!(probe.state.generates.lock().unwrap().is_empty());
    worker.submit(Action::Select("propio".into())).unwrap();
    assert!(matches!(wait(&mut worker), Event::Selected(_)));
    worker.submit(Action::Liberate).unwrap();
    match wait(&mut worker) {
        Event::Liberated(text) => {
            assert!(text.contains("no se descarga"), "{text}");
            assert!(text.contains("propio"), "{text}");
            assert!(text.contains("other"), "{text}");
            assert!(text.contains("medición NVIDIA no disponible"), "{text}");
        }
        other => panic!("liberación inesperada: {other:?}"),
    }
    assert!(
        probe.state.generates.lock().unwrap().is_empty(),
        "modo compartido no puede llamar a /api/generate"
    );
}

#[test]
fn exclusive_unloads_only_the_model_selected_in_this_session() {
    let probe = Probe::start(&["propio", "other"], true, 0);
    let session = session(&probe.url(), true);
    grant_exclusive(&session);
    let mut worker =
        Worker::start_project(&session.root, &session.config, None, &session.trust).unwrap();
    match wait(&mut worker) {
        Event::Notice { text, .. } => {
            assert!(text.contains("no hay modelos de esta sesión"), "{text}");
        }
        other => panic!("arranque inesperado: {other:?}"),
    }
    assert!(probe.state.generates.lock().unwrap().is_empty());
    worker.submit(Action::Select("propio".into())).unwrap();
    assert!(matches!(wait(&mut worker), Event::Selected(_)));
    worker.submit(Action::Liberate).unwrap();
    match wait(&mut worker) {
        Event::Liberated(text) => {
            assert!(text.contains("descargados: propio"), "{text}");
            assert!(text.contains("conservados: other"), "{text}");
            assert!(!text.contains("descargados: other"), "{text}");
        }
        other => panic!("liberación inesperada: {other:?}"),
    }
    let generates = probe.state.generates.lock().unwrap().clone();
    assert_eq!(generates.len(), 1, "{generates:?}");
    assert_eq!(generates[0]["model"], "propio");
    assert_eq!(generates[0]["keep_alive"], 0);
    assert_eq!(generates[0]["stream"], false);
    assert!(probe.state.loaded.lock().unwrap().contains("other"));
    assert!(!probe.state.loaded.lock().unwrap().contains("propio"));
}

#[test]
fn an_unconfirmed_unload_is_not_reported_as_success() {
    let probe = Probe::start(&["propio"], false, 0);
    let session = session(&probe.url(), true);
    grant_exclusive(&session);
    let mut worker =
        Worker::start_project(&session.root, &session.config, None, &session.trust).unwrap();
    assert!(matches!(wait(&mut worker), Event::Notice { .. }));
    worker.submit(Action::Select("propio".into())).unwrap();
    assert!(matches!(wait(&mut worker), Event::Selected(_)));
    worker.submit(Action::Liberate).unwrap();
    match wait(&mut worker) {
        Event::Failed(error) => assert!(error.contains("no se confirmó"), "{error}"),
        other => panic!("se anunció un éxito falso: {other:?}"),
    }
}

#[test]
fn requests_wait_until_liberation_finishes() {
    let probe = Probe::start(&["other"], true, 2);
    let session = session(&probe.url(), false);
    let mut worker =
        Worker::start_project(&session.root, &session.config, None, &session.trust).unwrap();
    assert!(matches!(wait(&mut worker), Event::Notice { .. }));
    worker.submit(Action::Liberate).unwrap();
    let start = Instant::now();
    while probe.state.ps_count.load(Ordering::SeqCst) < 2 {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("la liberación no llegó a /api/ps");
        }
        thread::sleep(Duration::from_millis(20));
    }
    let rejected = worker.submit(Action::Chat("espera".into())).unwrap_err();
    assert!(rejected.contains("liberación en curso"), "{rejected}");
    probe.state.released.store(true, Ordering::SeqCst);
    assert!(matches!(wait(&mut worker), Event::Liberated(_)));
    assert!(probe.state.generates.lock().unwrap().is_empty());
}
