//! R04: corta bytes de un flujo de Ollama real antes del final confirmado.
//! El proxy solo altera el transporte; nunca construye respuestas del modelo.

use std::{
    env, fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use pill::{
    config::Config,
    providers::ollama::{self, Ollama},
    session::{Action, Event, Worker},
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Default, Serialize)]
struct Trace {
    requests: Vec<Value>,
    upstream_ndjson: Vec<Value>,
    cut_line_original_bytes: Vec<u8>,
    cut_line_forwarded_bytes: Vec<u8>,
    cut_line_had_done: Option<bool>,
    errors: Vec<String>,
}

struct Proxy {
    address: std::net::SocketAddr,
    trace: Arc<Mutex<Trace>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Proxy {
    fn start(config: &pill::config::OllamaConfig) -> Self {
        let upstream = ollama::local_url(&config.url).expect("Ollama real debe usar loopback");
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let trace = Arc::new(Mutex::new(Trace::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_trace = trace.clone();
        let thread_stop = stop.clone();
        let config = config.clone();
        let handle = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_millis(config.connect_timeout_ms))
                .timeout(Duration::from_millis(config.response_timeout_ms))
                .build()
                .unwrap();
            while !thread_stop.load(Ordering::SeqCst) {
                let Ok((stream, _)) = listener.accept() else {
                    break;
                };
                if thread_stop.load(Ordering::SeqCst) {
                    break;
                }
                let _ =
                    stream.set_read_timeout(Some(Duration::from_millis(config.connect_timeout_ms)));
                let _ = stream
                    .set_write_timeout(Some(Duration::from_millis(config.connect_timeout_ms)));
                if let Err(error) =
                    runtime.block_on(forward(stream, &client, &upstream, &config, &thread_trace))
                {
                    thread_trace.lock().unwrap().errors.push(error);
                }
            }
        });
        Self {
            address,
            trace,
            stop,
            handle: Some(handle),
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.address)
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

fn request(stream: &mut TcpStream, limit: usize) -> Result<(String, String, Vec<u8>), String> {
    let mut bytes = Vec::new();
    let mut block = [0u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut block).map_err(|error| error.to_string())?;
        if count == 0 {
            return Err("petición HTTP incompleta".into());
        }
        bytes.extend_from_slice(&block[..count]);
        if bytes.len() > limit {
            return Err("petición supera límite configurado".into());
        }
        if let Some(index) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let header = std::str::from_utf8(&bytes[..header_end]).map_err(|error| error.to_string())?;
    let mut first = header.lines().next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("").to_owned();
    let path = first.next().unwrap_or("").to_owned();
    let length = header
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| {
            value
                .trim()
                .parse::<usize>()
                .map_err(|error| error.to_string())
        })
        .transpose()?
        .unwrap_or(0);
    if header_end.saturating_add(length) > limit {
        return Err("cuerpo supera límite configurado".into());
    }
    while bytes.len() < header_end + length {
        let remaining = (header_end + length - bytes.len()).min(block.len());
        let count = stream
            .read(&mut block[..remaining])
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Err("cuerpo HTTP incompleto".into());
        }
        bytes.extend_from_slice(&block[..count]);
    }
    Ok((
        method,
        path,
        bytes[header_end..header_end + length].to_vec(),
    ))
}

fn write_chunk(stream: &mut TcpStream, bytes: &[u8]) -> Result<(), String> {
    if !bytes.is_empty() {
        write!(stream, "{:x}\r\n", bytes.len()).map_err(|error| error.to_string())?;
        stream.write_all(bytes).map_err(|error| error.to_string())?;
        stream
            .write_all(b"\r\n")
            .map_err(|error| error.to_string())?;
    }
    stream.flush().map_err(|error| error.to_string())
}

/// Devuelve true solo cuando se observaron llamadas reales y se cortó el final.
fn forward_line(stream: &mut TcpStream, line: &[u8], trace: &Mutex<Trace>) -> Result<bool, String> {
    let value: Value = serde_json::from_slice(line).map_err(|error| error.to_string())?;
    let has_tools = value["message"]["tool_calls"]
        .as_array()
        .is_some_and(|calls| !calls.is_empty());
    trace.lock().unwrap().upstream_ndjson.push(value.clone());
    if !has_tools {
        write_chunk(stream, line)?;
        return Ok(false);
    }
    let done = value
        .get("done")
        .and_then(Value::as_bool)
        .ok_or("Ollama devolvió done no booleano")?;
    let forwarded = if done {
        // Prefijo estricto de la línea original: ni JSON completo ni done válido.
        let end = line
            .iter()
            .rposition(|byte| !byte.is_ascii_whitespace())
            .ok_or("línea vacía")?;
        &line[..end]
    } else {
        line
    };
    {
        let mut trace = trace.lock().unwrap();
        trace.cut_line_original_bytes = line.to_vec();
        trace.cut_line_forwarded_bytes = forwarded.to_vec();
        trace.cut_line_had_done = Some(done);
    }
    write_chunk(stream, forwarded)?;
    stream
        .write_all(b"0\r\n\r\n")
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    Ok(true)
}

async fn forward(
    mut stream: TcpStream,
    client: &reqwest::Client,
    upstream: &reqwest::Url,
    config: &pill::config::OllamaConfig,
    trace: &Mutex<Trace>,
) -> Result<(), String> {
    let (method, path, body) = request(&mut stream, config.max_response_bytes)?;
    if !matches!(
        path.as_str(),
        "/api/ps" | "/api/tags" | "/api/show" | "/api/chat"
    ) {
        return Err(format!("ruta no permitida en diagnóstico: {path}"));
    }
    let parsed_body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?
    };
    trace
        .lock()
        .unwrap()
        .requests
        .push(json!({"method":method,"path":path,"body":parsed_body}));
    let response = client
        .request(
            reqwest::Method::from_bytes(method.as_bytes()).map_err(|error| error.to_string())?,
            upstream.join(&path).map_err(|error| error.to_string())?,
        )
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json")
        .to_owned();
    write!(stream, "HTTP/1.1 {}\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n", status)
        .map_err(|error| error.to_string())?;
    let mut chunks = response.bytes_stream();
    let mut pending = Vec::new();
    let mut total = 0usize;
    while let Some(chunk) = chunks.next().await {
        let bytes = chunk.map_err(|error| error.to_string())?;
        total = total.saturating_add(bytes.len());
        if total > config.max_response_bytes {
            return Err("respuesta real supera límite configurado".into());
        }
        if path != "/api/chat" {
            write_chunk(&mut stream, &bytes)?;
            continue;
        }
        pending.extend_from_slice(&bytes);
        while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<_> = pending.drain(..=index).collect();
            if line.len() > config.max_line_bytes {
                return Err("línea real supera límite configurado".into());
            }
            if forward_line(&mut stream, &line, trace)? {
                return Ok(());
            }
        }
        if pending.len() > config.max_line_bytes {
            return Err("fragmento real supera límite configurado".into());
        }
    }
    if !pending.is_empty() && forward_line(&mut stream, &pending, trace)? {
        return Ok(());
    }
    stream
        .write_all(b"0\r\n\r\n")
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn wait(worker: &mut Worker, deadline: Instant, events: &mut Vec<String>) -> Result<Event, String> {
    loop {
        if let Some(event) = worker.next_event() {
            events.push(format!("{event:?}"));
            return Ok(event);
        }
        if Instant::now() >= deadline {
            return Err("el trabajador agotó el tiempo del diagnóstico".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "requiere Ollama real y modelo/evidencia explícitos; interrumpe solo el flujo del diagnóstico"]
fn real_tool_stream_interruption_never_executes_unconfirmed_calls() {
    let model = env::var("PILL_LIVE_MODEL").expect("modelo explícito");
    let evidence = env::var("PILL_LIVE_EVIDENCE").expect("ruta de evidencia explícita");
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("Proyecto ñ");
    fs::create_dir(&root).unwrap();
    let original = b"contenido del usuario\r\n";
    fs::write(root.join("conservar.txt"), original).unwrap();
    let mut config = Config::load(&root, None).unwrap().config;
    if let Ok(value) = env::var("PILL_LIVE_URL") {
        config.ollama.url = value;
    }
    if let Ok(value) = env::var("PILL_LIVE_CONTEXT") {
        config.ollama.num_ctx = value.parse().unwrap();
    }
    if let Ok(value) = env::var("PILL_LIVE_PREDICT") {
        config.ollama.num_predict = value.parse().unwrap();
    }
    if let Ok(value) = env::var("PILL_LIVE_THINK") {
        config.ollama.think = value.parse().unwrap();
    }
    let upstream_url = config.ollama.url.clone();
    // Valida por el mismo cliente que producción antes de iniciar el proxy.
    let _api = Ollama::new(&config.ollama).unwrap();
    let proxy = Proxy::start(&config.ollama);
    config.ollama.url = proxy.url();
    let trust = temporary.path().join("privado/trust.json");
    let mut worker = Worker::start_project(&root, &config, None, &trust).unwrap();
    let deadline = Instant::now()
        + Duration::from_millis(config.ollama.response_timeout_ms)
        + Duration::from_millis(config.ollama.connect_timeout_ms);
    let mut events = Vec::new();
    let mut failure = None;
    let mut tool_events = 0;
    let mut completed = false;
    let outcome = (|| -> Result<(), String> {
        loop {
            match wait(&mut worker, deadline, &mut events)? {
                Event::Notice { .. } => break,
                Event::Failed(error) => return Err(error),
                _ => {}
            }
        }
        worker.submit(Action::Select(model.clone()))?;
        loop {
            match wait(&mut worker, deadline, &mut events)? {
                Event::Selected(_) => break,
                Event::Failed(error) => return Err(error),
                _ => {}
            }
        }
        worker.submit(Action::Chat("Usa la herramienta create_file para crear solamente prueba_stream.txt con el contenido literal PRUEBA_STREAM. Haz la llamada a create_file ahora. No leas ni modifiques conservar.txt; no ejecutes comandos. No respondas con una propuesta: solicita la herramienta.".into()))?;
        loop {
            match wait(&mut worker, deadline, &mut events)? {
                Event::Tool { .. } => tool_events += 1,
                Event::Failed(error) => {
                    failure = Some(error);
                    break;
                }
                Event::Complete { .. } => {
                    completed = true;
                    break;
                }
                Event::Cancelled => return Err("cancelación inesperada".into()),
                _ => {}
            }
        }
        Ok(())
    })();
    drop(worker);
    let journals: Vec<String> = fs::read_dir(root.join(".pillcli/sessions"))
        .unwrap()
        .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();
    let requested_tools = journals
        .iter()
        .flat_map(|text| text.lines())
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|value| value["stage"] == "requested")
        .count();
    let original_after = fs::read(root.join("conservar.txt")).unwrap();
    let new_file_exists = root.join("prueba_stream.txt").exists();
    let trace = proxy.trace.lock().unwrap();
    let success = outcome.is_ok()
        && trace.cut_line_had_done.is_some()
        && trace.errors.is_empty()
        && failure.is_some()
        && !completed
        && tool_events == 0
        && requested_tools == 0
        && !new_file_exists
        && original_after == original;
    let report = json!({"result":if success {"PASS"} else {"FAIL"},"model":model,
        "upstream_url":upstream_url,"proxy_url":config.ollama.url,
        "config":{"num_ctx":config.ollama.num_ctx,"num_predict":config.ollama.num_predict,"think":config.ollama.think},
        "trace":*trace,"events":events,"diagnostic_error":outcome.err(),"worker_failure":failure,
        "completed":completed,"tool_events":tool_events,"requested_tools_in_journal":requested_tools,
        "fixture_sha256_before":format!("{:x}",Sha256::digest(original)),
        "fixture_sha256_after":format!("{:x}",Sha256::digest(&original_after)),
        "new_file_exists":new_file_exists,"journals":journals,
        "scope":"Ollama local real a través de proxy de transporte; únicamente bytes originales, cortados antes de un final válido; no respuestas inventadas, descargas, descarga de modelos ni efectos fuera del proyecto descartable"});
    fs::write(evidence, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    drop(trace);
    assert!(success, "{report}");
}
