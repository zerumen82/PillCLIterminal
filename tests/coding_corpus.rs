//! Corpus opt-in con modelo local y archivos reales descartables.
//! No sustituye respuestas del modelo ni modifica las pruebas para lograr PASS.
#![cfg(windows)]

use pill::{
    config::Config,
    session::{Action, Event, Worker},
    tools::commands::{CommandRunner, CommandSpec},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

struct Source {
    name: &'static str,
    before: &'static [u8],
    after: &'static [u8],
}

struct Case {
    name: &'static str,
    sources: [Source; 2],
    test: &'static str,
    instruction: &'static str,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn next(worker: &mut Worker, wait: Duration) -> Result<Event, String> {
    let start = Instant::now();
    while start.elapsed() < wait {
        if let Some(event) = worker.next_event() {
            return Ok(event);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err("el trabajador no emitió evento dentro del plazo de la prueba".into())
}

fn wait_start(worker: &mut Worker, selected: bool, wait: Duration) -> Result<(), String> {
    loop {
        match next(worker, wait)? {
            Event::Notice { .. } if !selected => return Ok(()),
            Event::Selected(_) if selected => return Ok(()),
            Event::Budget { .. } | Event::Activity { .. } | Event::Restored { .. } => {}
            event => return Err(format!("arranque/selección: {event:?}")),
        }
    }
}

fn run_case(case: &Case, model: &str, python: &Path) -> Value {
    let sandbox = tempfile::tempdir().unwrap();
    let root = sandbox.path().join(format!("Proyecto ñ {}", case.name));
    fs::create_dir(&root).unwrap();
    for source in &case.sources {
        fs::write(root.join(source.name), source.before).unwrap();
    }
    let test_path = root.join("verify.py");
    fs::write(&test_path, case.test.as_bytes()).unwrap();
    let test_sha = hash(case.test.as_bytes());
    let mut config = Config::load(&root, None).unwrap().config;
    if let Ok(value) = env::var("PILL_LIVE_CONTEXT") {
        config.ollama.num_ctx = value.parse().expect("contexto explícito válido");
    }
    if let Ok(value) = env::var("PILL_LIVE_PREDICT") {
        config.ollama.num_predict = value.parse().expect("límite explícito válido");
    }
    if let Ok(value) = env::var("PILL_LIVE_THINK") {
        config.ollama.think = value.parse().expect("thinking explícito válido");
    }
    // El modelo no puede editar ni leer la prueba; el ejecutor concedido sí la ejecuta.
    config.files.excluded_components.push("verify.py".into());
    config.commands.commands = vec![CommandSpec {
        id: "verify_corpus".into(),
        program: python.to_path_buf(),
        args: vec!["-B".into(), "verify.py".into()],
        cwd: ".".into(),
        watched_files: vec!["verify.py".into()],
    }];
    let trust = sandbox.path().join("private/trust.json");
    let runner = CommandRunner::new(&root, config.commands.clone(), &trust).unwrap();
    let proposal = runner.proposals().unwrap().remove(0);
    runner
        .grant("verify_corpus", &proposal.fingerprint)
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let execute = || {
        runtime
            .block_on(runner.execute("verify_corpus", Arc::new(AtomicBool::new(false))))
            .unwrap()
    };
    let before = execute();
    let mut worker = Worker::start_project(&root, &config, None, &trust).unwrap();
    let wait = Duration::from_millis(config.ollama.response_timeout_ms.saturating_add(10_000));
    let mut errors = Vec::new();
    let mut tools = Vec::new();
    let mut notices = Vec::new();
    let mut answer = String::new();
    let mut completed = false;
    let started = Instant::now();
    let result = (|| -> Result<(), String> {
        wait_start(&mut worker, false, wait)?;
        worker
            .submit(Action::Select(model.into()))
            .map_err(|e| e.to_string())?;
        wait_start(&mut worker, true, wait)?;
        let prompt = format!(
            "Corrige los dos archivos indicados usando herramientas reales. {} Lee ambos con read_file y usa sus SHA reales en edit_file o edit_file_many. Haz únicamente los reemplazos mínimos y conserva BOM y saltos existentes. verify.py está protegida; no la edites ni crees pruebas alternativas. Tras corregir ambos archivos ejecuta run_command con id verify_corpus y comprueba que termina con código 0. No declares cambios sin ejecutarlos.",
            case.instruction
        );
        worker
            .submit(Action::Chat(prompt))
            .map_err(|e| e.to_string())?;
        loop {
            match next(&mut worker, wait)? {
                Event::Delta(text) => answer.push_str(&text),
                Event::Tool {
                    role,
                    call_id,
                    name,
                    arguments,
                    result,
                    journal,
                } => {
                    tools.push(json!({"role":role,"call_id":call_id,"name":name,"arguments":arguments,"result":result,"journal":journal}));
                }
                Event::Notice { role, text } => notices.push(json!({"role":role,"text":text})),
                Event::Complete {
                    answer: final_answer,
                    ..
                } => {
                    completed = true;
                    if let Some(text) = final_answer {
                        answer = text;
                    }
                    return Ok(());
                }
                Event::Budget { .. }
                | Event::Activity { .. }
                | Event::Reasoning { .. }
                | Event::Restored { .. } => {}
                event => return Err(format!("generación: {event:?}")),
            }
        }
    })();
    if let Err(error) = result {
        errors.push(error);
    }
    drop(worker);
    let after = execute();
    let mut source_reports = Vec::new();
    for source in &case.sources {
        let actual = fs::read(root.join(source.name)).unwrap();
        let edits: Vec<_> = tools
            .iter()
            .filter(|tool| {
                matches!(tool["name"].as_str(), Some("edit_file" | "edit_file_many"))
                    && tool["arguments"]["path"] == source.name
                    && tool["result"]["status"] == "verified"
            })
            .collect();
        let backup_checks: Vec<_> = edits.iter().map(|tool| {
            let result = &tool["result"];
            let backup = result["backup_path"].as_str().and_then(|path|fs::read(path).ok());
            json!({"backup_path":result["backup_path"],"expected_sha256":result["backup_sha256"],
                "actual_sha256":backup.as_ref().map(|bytes|hash(bytes)),
                "valid":backup.as_ref().is_some_and(|bytes|result["backup_sha256"]==hash(bytes)),
                "original_preserved":backup.as_deref()==Some(source.before)})
        }).collect();
        let read_before_edit = tools.iter().any(|tool| {
            tool["name"] == "read_file"
                && tool["arguments"]["path"] == source.name
                && tool["result"]["sha256"] == hash(source.before)
        });
        let last_sha_matches = edits
            .last()
            .is_some_and(|tool| tool["result"]["sha256"] == hash(&actual));
        let success = actual == source.after
            && read_before_edit
            && last_sha_matches
            && !backup_checks.is_empty()
            && backup_checks.iter().all(|b| b["valid"] == true)
            && backup_checks
                .iter()
                .any(|b| b["original_preserved"] == true);
        source_reports.push(json!({"path":source.name,"result":if success{"PASS"}else{"FAIL"},"before_sha256":hash(source.before),"after_sha256":hash(&actual),"expected_after_sha256":hash(source.after),"actual_bytes":actual,"read_before_edit":read_before_edit,"last_observed_sha_matches_disk":last_sha_matches,"backups":backup_checks}));
    }
    let test_unchanged = hash(&fs::read(&test_path).unwrap()) == test_sha;
    let model_test_verified = tools
        .iter()
        .any(|tool| tool["name"] == "run_command" && tool["result"]["status"] == "verified");
    let journals: Vec<_> = fs::read_dir(root.join(".pillcli/sessions")).unwrap()
        .map(|entry| {let path=entry.unwrap().path();json!({"name":path.file_name().unwrap().to_string_lossy(),"bytes":fs::read(&path).unwrap()})}).collect();
    let success = completed
        && errors.is_empty()
        && before.exit_code != Some(0)
        && before.stdout.contains("A_FAIL")
        && before.stdout.contains("B_FAIL")
        && after.exit_code == Some(0)
        && after.stdout.contains("CORPUS_PASS")
        && test_unchanged
        && model_test_verified
        && source_reports.iter().all(|item| item["result"] == "PASS");
    json!({"name":case.name,"result":if success{"PASS"}else{"FAIL"},"model":model,
        "config":{"num_ctx":config.ollama.num_ctx,"num_predict":config.ollama.num_predict,"think":config.ollama.think},
        "before":before,"after":after,"completed":completed,"errors":errors,"test_unchanged":test_unchanged,
        "test_sha256":test_sha,"sources":source_reports,"tools":tools,"notices":notices,"response":answer,
        "elapsed_ms":started.elapsed().as_millis(),"journals":journals})
}

#[test]
#[ignore = "requiere Ollama, modelo/Python/evidencia explícitos; dos tareas y cuatro archivos descartables"]
fn real_model_repairs_two_multi_file_tasks_preserving_bom_crlf_and_backups() {
    let model = env::var("PILL_LIVE_MODEL").expect("modelo explícito");
    let evidence =
        PathBuf::from(env::var_os("PILL_LIVE_EVIDENCE").expect("ruta de evidencia explícita"));
    assert!(!evidence.exists(), "no se sobrescriben evidencias previas");
    let python = PathBuf::from(env::var_os("PILL_TEST_PYTHON").expect("Python explícito"))
        .canonicalize()
        .unwrap();
    let cases = [
        Case {
            name: "aritmetica-bom-crlf",
            sources: [
                Source {
                    name: "math_ops.py",
                    before: b"\xef\xbb\xbfdef suma(a, b):\r\n    return a - b\r\n",
                    after: b"\xef\xbb\xbfdef suma(a, b):\r\n    return a + b\r\n",
                },
                Source {
                    name: "geometry.py",
                    before: b"\xef\xbb\xbfdef perimetro(a, b):\r\n    return a + b\r\n",
                    after: b"\xef\xbb\xbfdef perimetro(a, b):\r\n    return 2 * (a + b)\r\n",
                },
            ],
            instruction: "En math_ops.py, suma debe sumar: cambia a - b por a + b. En geometry.py, perimetro calcula un rectángulo: cambia return a + b por return 2 * (a + b).",
            test: "import math_ops, geometry\na = math_ops.suma(2, 3) == 5 and math_ops.suma(-2, 3) == 1\nb = geometry.perimetro(2, 3) == 10 and geometry.perimetro(0, 4) == 8\nprint('A_PASS' if a else 'A_FAIL')\nprint('B_PASS' if b else 'B_FAIL')\nassert a and b\nprint('CORPUS_PASS')\n",
        },
        Case {
            name: "texto-lf",
            sources: [
                Source {
                    name: "normalize.py",
                    before: b"def normaliza(text):\n    return text.strip()\n",
                    after: b"def normaliza(text):\n    return text.strip().lower()\n",
                },
                Source {
                    name: "joiner.py",
                    before: b"def une(words):\n    return ''.join(words)\n",
                    after: b"def une(words):\n    return ' '.join(words)\n",
                },
            ],
            instruction: "En normalize.py, normaliza debe quitar espacios laterales y pasar a minúsculas: cambia text.strip() por text.strip().lower(). En joiner.py, une debe separar palabras con un espacio: cambia ''.join(words) por ' '.join(words).",
            test: "import normalize, joiner\na = normalize.normaliza('  ÁRBOL  ') == 'árbol' and normalize.normaliza('') == ''\nb = joiner.une(['uno', 'dos']) == 'uno dos' and joiner.une([]) == ''\nprint('A_PASS' if a else 'A_FAIL')\nprint('B_PASS' if b else 'B_FAIL')\nassert a and b\nprint('CORPUS_PASS')\n",
        },
    ];
    let reports: Vec<_> = cases
        .iter()
        .map(|case| run_case(case, &model, &python))
        .collect();
    let success = reports.iter().all(|report| report["result"] == "PASS");
    let report = json!({"result":if success{"PASS"}else{"FAIL"},"cases":reports,"scope":"Dos tareas distintas, cuatro archivos reales, BOM/CRLF y LF, modelo local y herramientas reales, pruebas protegidas antes/después, SHA y respaldos observados; no es evaluación estadística general del modelo"});
    fs::write(&evidence, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(success, "corpus falló; evidencia: {}", evidence.display());
}
