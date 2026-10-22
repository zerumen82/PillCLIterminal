//! Integración opt-in contra el servidor y modelo reales seleccionados por entorno.
use pill::{
    config::Config,
    providers::ollama::{Message, Ollama},
    session::{Action, Event, Worker},
};
use serde_json::json;
use std::{env, fs};

#[test]
#[ignore = "requiere Ollama real; PILL_LIVE_MODEL y PILL_LIVE_EVIDENCE explícitos"]
fn real_chat_and_errors() {
    let name = env::var("PILL_LIVE_MODEL").expect("modelo explícito");
    let path = env::var("PILL_LIVE_EVIDENCE").expect("ruta explícita para evidencia");
    let loaded = Config::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR")), None).unwrap();
    let config = loaded.config.ollama;
    let api = Ollama::new(&config).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let before = api.running().await.unwrap();
        let selected = api.select(&name).await.unwrap();
        let messages = vec![Message::new("system", &config.system_prompt), Message::new("user", "Escribe solamente una función Python llamada suma que devuelva a + b. Sin explicaciones.")];
        let mut deltas = Vec::new();
        let outcome = api.chat(&selected, &messages, |update| {
            if let pill::providers::ollama::StreamUpdate::Text(text) = update {
                deltas.push(text);
            }
            std::future::ready(Ok(()))
        }).await.unwrap();
        assert_eq!(deltas.concat(), outcome.content);
        assert!(deltas.len() > 1, "debe observarse streaming real");
        assert!(outcome.content.contains("suma"));
        let after = api.running().await.unwrap();
        let missing = api.select("pill-nonexistent-test-model:missing").await.unwrap_err().to_string();
        assert!(missing.contains("no instalado"));
        fs::write(&path, serde_json::to_vec_pretty(&json!({
            "model":selected,"config":{"url":config.url,"num_ctx":config.num_ctx,"num_predict":config.num_predict,"temperature":config.temperature,"think":config.think},
            "prompt":messages,"outcome":outcome,"delta_count":deltas.len(),
            "running_before":before,"running_after":after,"missing_model_error":missing,
            "scope":"Inferencia y streaming reales; no acredita calidad general ni herramientas ni exclusividad del servidor"
        })).unwrap()).unwrap();
    });
}

#[test]
#[ignore = "requiere Ollama real; usa el modelo explícito y cancela sin descargarlo"]
fn real_cancel_and_retry() {
    let name = env::var("PILL_LIVE_MODEL").expect("modelo explícito");
    let path = env::var("PILL_LIVE_EVIDENCE").expect("ruta explícita para evidencia");
    let loaded = Config::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR")), None).unwrap();
    let mut worker = Worker::start(loaded.config.ollama).unwrap();
    fn next(worker: &mut Worker) -> Event {
        let start = std::time::Instant::now();
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
            assert!(start.elapsed().as_secs() < 240, "sin evento del trabajador");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    worker.submit(Action::Select(name.clone())).unwrap();
    assert!(matches!(next(&mut worker), Event::Selected(_)));
    worker
        .submit(Action::Chat(
            "Enumera los números desde uno hasta mil con explicación de cada uno.".into(),
        ))
        .unwrap();
    let first_delta = match next(&mut worker) {
        Event::Delta(text) => text,
        other => panic!("esperaba primer fragmento: {other:?}"),
    };
    let cancel_start = std::time::Instant::now();
    worker.submit(Action::Cancel).unwrap();
    let mut queued_deltas = 0;
    loop {
        match next(&mut worker) {
            Event::Delta(_) => queued_deltas += 1,
            Event::Cancelled => break,
            other => panic!("esperaba cancelación: {other:?}"),
        }
    }
    let cancel_elapsed = cancel_start.elapsed().as_secs_f64();
    let retry_start = std::time::Instant::now();
    worker
        .submit(Action::Chat(
            "Escribe exactamente: REINTENTO_CORRECTO".into(),
        ))
        .unwrap();
    let mut answer = String::new();
    loop {
        match next(&mut worker) {
            Event::Delta(text) => answer.push_str(&text),
            Event::Complete { .. } => break,
            other => panic!("reintento falló: {other:?}"),
        }
    }
    assert!(answer.contains("REINTENTO_CORRECTO"));
    let retry_elapsed = retry_start.elapsed().as_secs_f64();
    // También cancelar inmediatamente después de encolar y salir debe terminar sin esperar generación.
    worker
        .submit(Action::Chat("Enumera los números hasta mil.".into()))
        .unwrap();
    let start = std::time::Instant::now();
    drop(worker);
    assert!(start.elapsed().as_secs() < 3);
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "model": name,
            "first_delta_before_cancel": first_delta,
            "queued_deltas_before_cancelled": queued_deltas,
            "cancel_elapsed_seconds": cancel_elapsed,
            "retry_answer": answer,
            "retry_elapsed_seconds": retry_elapsed,
            "drop_elapsed_seconds": start.elapsed().as_secs_f64(),
            "result": "PASS",
            "scope": "Cancelación tras primer fragmento real y reintento; no prueba descarga ni herramientas."
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requiere metadatos de Ollama real; no genera ni descarga modelos"]
fn real_connection_and_context_errors() {
    let name = env::var("PILL_LIVE_MODEL").expect("modelo explícito");
    let path = env::var("PILL_LIVE_EVIDENCE").expect("ruta explícita para evidencia");
    let loaded = Config::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR")), None).unwrap();
    let mut config = loaded.config.ollama;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let selected = Ollama::new(&config).unwrap().select(&name).await.unwrap();
        config.num_ctx = u32::try_from(selected.context_limit).unwrap().checked_add(1).unwrap();
        let context_error = Ollama::new(&config).unwrap().select(&name).await.unwrap_err();
        assert!(context_error.to_string().contains("supera el contexto anunciado"));
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        config.url = format!("http://{address}");
        let connection_error = Ollama::new(&config).unwrap().models().await.unwrap_err();
        assert!(matches!(connection_error, pill::providers::ollama::Error::Http(_)));
        fs::write(path, serde_json::to_vec_pretty(&json!({
            "model": selected,
            "requested_context": config.num_ctx,
            "context_error": context_error.to_string(),
            "closed_loopback_url": config.url,
            "connection_error": connection_error.to_string(),
            "result": "PASS",
            "scope": "Metadatos reales, contexto anunciado excedido y conexión rechazada real; sin generación ni servidor simulado."
        })).unwrap()).unwrap();
    });
}
