//! Fallos de transporte reales en loopback; no son inferencias de Ollama.
use pill::{
    config::Config,
    providers::ollama::{Error, Message, Model, Ollama, Phase, SelectedModel},
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    count: Arc<AtomicUsize>,
    chats: Arc<AtomicUsize>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn start(mode: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let count = Arc::new(AtomicUsize::new(0));
        let chats = Arc::new(AtomicUsize::new(0));
        let generated = chats.clone();
        let stopped = stop.clone();
        let observed = count.clone();
        let handle = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let Ok((mut socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let Some(path) = request(&mut socket) else {
                    continue;
                };
                if path == "/api/tags" {
                    let attempt = observed.fetch_add(1, Ordering::SeqCst);
                    let (status, body) = if mode == "retry" && attempt == 0 {
                        (503, r#"{"error":"transitorio"}"#)
                    } else {
                        (
                            200,
                            r#"{"models":[{"name":"sample","digest":"observed-digest","size":1}]}"#,
                        )
                    };
                    let _ = write!(
                        socket,
                        "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                } else if path == "/api/ps" {
                    let body = r#"{"models":null}"#;
                    let _ = write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                } else if path == "/api/chat" {
                    generated.fetch_add(1, Ordering::SeqCst);
                    if mode == "first" {
                        thread::sleep(Duration::from_millis(250));
                        continue;
                    }
                    let _=socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n");
                    let line=b"{\"done\":false,\"message\":{\"role\":\"assistant\",\"content\":\"dato real recibido\"}}\n";
                    let _ = socket.write_all(line);
                    if mode == "idle" {
                        thread::sleep(Duration::from_millis(250));
                    } else {
                        for _ in 0..50 {
                            thread::sleep(Duration::from_millis(40));
                            if socket.write_all(line).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });
        Self {
            url: format!("http://{address}"),
            stop,
            count,
            chats,
            handle: Some(handle),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            h.join().unwrap();
        }
    }
}
fn request(stream: &mut TcpStream) -> Option<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    let header = loop {
        let n = stream.read(&mut buffer).ok()?;
        if n == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(p) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        if bytes.len() > 65536 {
            return None;
        }
    };
    let text = String::from_utf8_lossy(&bytes[..header]);
    let path = text.lines().next()?.split_whitespace().nth(1)?.to_owned();
    let length = text
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|s| s.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    while bytes.len() < header + length {
        let n = stream.read(&mut buffer).ok()?;
        if n == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    Some(path)
}
fn configuration(server: &Server) -> pill::config::OllamaConfig {
    let mut config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    config.ollama.url = server.url.clone();
    config.ollama.retry_delay_ms = 1;
    config.ollama
}
#[test]
fn only_read_metadata_retries_transient_status() {
    let server = Server::start("retry");
    let api = Ollama::new(&configuration(&server)).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let models = rt.block_on(api.models()).unwrap();
    assert_eq!(models[0].digest, "observed-digest");
    assert_eq!(server.count.load(Ordering::SeqCst), 2);
}
#[test]
fn generation_timeouts_have_distinct_phases_and_never_confirm_partial_output() {
    for (mode, expected) in [
        ("first", Phase::FirstChunk),
        ("idle", Phase::Inactivity),
        ("total", Phase::Total),
    ] {
        let server = Server::start(mode);
        let mut config = configuration(&server);
        config.connect_timeout_ms = 100;
        config.first_chunk_timeout_ms = 100;
        config.idle_timeout_ms = 100;
        config.response_timeout_ms = 1000;
        let api = Ollama::new(&config).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let model = SelectedModel {
            model: Model {
                name: "sample".into(),
                digest: "observed-digest".into(),
                size: 1,
            },
            capabilities: vec!["completion".into()],
            context_limit: 8192,
        };
        let result = rt.block_on(api.chat(&model, &[Message::new("user", "prueba")], |_| {
            std::future::ready(Ok(()))
        }));
        assert!(
            matches!(result,Err(Error::Timeout{phase}) if phase==expected),
            "{mode}: {result:?}"
        );
        assert_eq!(
            server.count.load(Ordering::SeqCst),
            1,
            "no repetir generación al fallar"
        );
        assert_eq!(
            server.chats.load(Ordering::SeqCst),
            1,
            "no repetir generación al fallar"
        );
    }
}
#[test]
fn default_tool_catalog_has_a_reported_budget() {
    let config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    let used = pill::providers::ollama::reserved_context(
        &[
            Message::new("system", &config.ollama.system_prompt),
            Message::new("user", "hola"),
        ],
        &pill::tools::schemas(pill::tools::Role::Implementer, false),
        None,
        config.ollama.num_predict,
    )
    .unwrap();
    assert!(
        used <= config.ollama.num_ctx as usize,
        "el catálogo incluido no cabe ni en el primer turno: {used}/{}",
        config.ollama.num_ctx
    );
}

#[test]
fn invalid_running_metadata_is_unknown_in_model_list_and_status() {
    use pill::session::{Action, Event, Worker};
    let server = Server::start("bad_ps");
    let mut worker = Worker::start(configuration(&server)).unwrap();
    let next = |worker: &mut Worker| {
        let start = std::time::Instant::now();
        loop {
            if let Some(event) = worker.next_event()
                && !matches!(event, Event::Notice { .. } | Event::Budget { .. })
            {
                break event;
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(5));
        }
    };
    worker.submit(Action::Discover).unwrap();
    match next(&mut worker) {
        Event::Models(models) => assert_eq!(models[0].loaded, None),
        other => panic!("{other:?}"),
    };
    worker.submit(Action::Status).unwrap();
    match next(&mut worker) {
        Event::Status { report } => {
            assert!(report["model"]["loaded"].is_null());
            assert!(
                report["connection_error"]
                    .as_str()
                    .unwrap()
                    .contains("/api/ps")
            );
        }
        other => panic!("{other:?}"),
    };
}
