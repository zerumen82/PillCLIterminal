//! R13 opt-in: servidor Ollama exclusivo creado por el diagnóstico, nunca el compartido.

use std::{
    env, fs,
    time::{Duration, Instant},
};

use pill::{
    config::Config,
    providers::ollama::{self, Ollama},
    session::{Action, Event, Worker},
};
use serde_json::{Value, json};

fn next(worker: &mut Worker, deadline: Instant, events: &mut Vec<String>) -> Result<Event, String> {
    loop {
        if let Some(event) = worker.next_event() {
            events.push(format!("{event:?}"));
            if matches!(
                event,
                Event::Budget { .. }
                    | Event::Activity { .. }
                    | Event::Reasoning { .. }
                    | Event::Restored { .. }
                    | Event::TaskStep { .. }
                    | Event::Delta(_)
            ) {
                continue;
            }
            return Ok(event);
        }
        if Instant::now() >= deadline {
            return Err("timeout del diagnóstico exclusivo".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn model_loaded(running: &Value, name: &str, digest: &str) -> bool {
    running["models"].as_array().is_some_and(|models| {
        models
            .iter()
            .any(|model| model["name"] == name && model["digest"] == digest)
    })
}

#[test]
#[ignore = "requiere servidor propio PILL_RELEASE_URL y modelo/evidencia explícitos; descarga únicamente su modelo de prueba"]
fn real_exclusive_release_preserves_shared_mode_and_unloads_owned_model() {
    let name = env::var("PILL_LIVE_MODEL").expect("modelo instalado explícito");
    let evidence = env::var("PILL_LIVE_EVIDENCE").expect("evidencia explícita");
    let release_url = env::var("PILL_RELEASE_URL").expect("URL de servidor propio explícita");
    let url = ollama::local_url(&release_url).expect("servidor exclusivo debe ser HTTP loopback");
    assert_eq!(
        url.host_str(),
        Some("127.0.0.1"),
        "diagnóstico solo admite el listener IPv4 propio"
    );
    assert!(
        url.port().is_some_and(|port| port != 11434),
        "nunca descargar del puerto habitual compartido"
    );
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("Proyecto ñ");
    fs::create_dir(&root).unwrap();
    let mut config = Config::load(&root, None).unwrap().config;
    assert_ne!(
        release_url, config.ollama.url,
        "no reutilizar el servidor predeterminado"
    );
    config.ollama.url = release_url;
    if let Ok(value) = env::var("PILL_LIVE_CONTEXT") {
        config.ollama.num_ctx = value.parse().unwrap();
    }
    if let Ok(value) = env::var("PILL_LIVE_PREDICT") {
        config.ollama.num_predict = value.parse().unwrap();
    }
    if let Ok(value) = env::var("PILL_LIVE_THINK") {
        config.ollama.think = value.parse().unwrap();
    }
    config.gpu.exclusive = true;
    let trust = temporary.path().join("privado/trust.json");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let api = Ollama::new(&config.ollama).unwrap();
    let before = rt.block_on(api.running()).unwrap();
    assert!(
        before["models"].as_array().is_some_and(Vec::is_empty),
        "el servidor propio debe comenzar sin modelos; no reclamar recursos preexistentes"
    );
    let tags_before = rt.block_on(api.models()).unwrap();
    let mut worker = Worker::start_project(&root, &config, None, &trust).unwrap();
    let mut events = Vec::new();
    let mut selected_digest = String::new();
    let mut loaded = Value::Null;
    let mut conserved = Value::Null;
    let mut after = Value::Null;
    let mut shared_report = String::new();
    let mut exclusive_report = String::new();
    let deadline = Instant::now()
        + Duration::from_millis(config.ollama.response_timeout_ms)
        + Duration::from_millis(config.ollama.connect_timeout_ms);
    let result = (|| -> Result<(), String> {
        match next(&mut worker, deadline, &mut events)? {
            Event::Notice { text, .. } if text.contains("no se descarga") => {}
            event => return Err(format!("arranque sin concesión inesperado: {event:?}")),
        }
        if trust.exists() {
            return Err("arrancar no puede conceder exclusividad".into());
        }
        worker.submit(Action::Select(name.clone()))?;
        match next(&mut worker, deadline, &mut events)? {
            Event::Selected(selected) => selected_digest = selected.model.digest,
            event => return Err(format!("selección inesperada: {event:?}")),
        }
        worker.submit(Action::Chat(
            "Responde solamente PRUEBA_GPU_LOCAL. No uses herramientas ni modifiques archivos."
                .into(),
        ))?;
        loop {
            match next(&mut worker, deadline, &mut events)? {
                Event::Notice { .. } => continue,
                Event::Complete { .. } => break,
                event => return Err(format!("chat real inesperado: {event:?}")),
            }
        }
        loaded = rt
            .block_on(api.running())
            .map_err(|error| error.to_string())?;
        if !model_loaded(&loaded, &name, &selected_digest) {
            return Err("el chat no dejó cargado el modelo real seleccionado".into());
        }
        worker.submit(Action::Liberate)?;
        match next(&mut worker, deadline, &mut events)? {
            Event::Liberated(text) => shared_report = text,
            event => return Err(format!("liberación compartida inesperada: {event:?}")),
        }
        conserved = rt
            .block_on(api.running())
            .map_err(|error| error.to_string())?;
        if !shared_report.contains("no se descarga")
            || !model_loaded(&conserved, &name, &selected_digest)
        {
            return Err("el modo compartido no conservó el modelo real".into());
        }
        worker.submit(Action::Permissions)?;
        let offer = match next(&mut worker, deadline, &mut events)? {
            Event::Permissions(offers) => offers
                .into_iter()
                .find(|offer| offer.id == "exclusive")
                .ok_or("falta propuesta explícita de exclusividad")?,
            event => return Err(format!("propuestas inesperadas: {event:?}")),
        };
        // Consentimiento concreto del harness para su propio servidor y trust temporal.
        worker.submit(Action::Grant {
            id: offer.id,
            fingerprint: offer.fingerprint,
        })?;
        match next(&mut worker, deadline, &mut events)? {
            Event::Granted(_) => {}
            event => return Err(format!("concesión inesperada: {event:?}")),
        }
        worker.submit(Action::Liberate)?;
        match next(&mut worker, deadline, &mut events)? {
            Event::Liberated(text) => exclusive_report = text,
            event => return Err(format!("descarga exclusiva inesperada: {event:?}")),
        }
        after = rt
            .block_on(api.running())
            .map_err(|error| error.to_string())?;
        if model_loaded(&after, &name, &selected_digest)
            || !exclusive_report.contains(&format!("descargados: {name}"))
        {
            return Err("no se confirmó la descarga exclusiva real".into());
        }
        Ok(())
    })();
    drop(worker);
    let tags_after = rt.block_on(api.models()).unwrap();
    let digests = |models: &[pill::providers::ollama::Model]| {
        models
            .iter()
            .map(|model| (model.name.clone(), model.digest.clone()))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let installed_models_unchanged = digests(&tags_before) == digests(&tags_after);
    let model_vram_before = loaded["models"]
        .as_array()
        .and_then(|models| models.iter().find(|model| model["name"] == name))
        .and_then(|model| model["size_vram"].as_u64());
    let success = result.is_ok() && installed_models_unchanged;
    let report = json!({"result":if success {"PASS"} else {"FAIL"},"model":name,
        "digest":selected_digest,"exclusive_url":config.ollama.url,
        "config":{"num_ctx":config.ollama.num_ctx,"num_predict":config.ollama.num_predict,"think":config.ollama.think},
        "running_before":before,"running_after_real_chat":loaded,"running_after_shared_release":conserved,
        "running_after_exclusive_release":after,"shared_report":shared_report,"exclusive_report":exclusive_report,
        "selected_model_size_vram_before":model_vram_before,"model_was_on_gpu":model_vram_before.is_some_and(|bytes| bytes>0),
        "installed_models_unchanged":installed_models_unchanged,"events":events,"error":result.err(),
        "scope":"Servidor propio en puerto loopback distinto del habitual; concesión temporal explícita; chat real, conservación compartida y descarga exclusiva confirmada por /api/ps. La VRAM global se registra por separado; no se atribuye su delta exclusivamente a este modelo."});
    fs::write(evidence, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(success, "{report}");
}
