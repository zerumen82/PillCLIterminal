//! Pruebas opt-in: fixture real, modelo local y ejecutor del producto.
use pill::{
    config::Config,
    session::{Action, Event, Worker},
    tools::commands::{CommandRunner, CommandSpec},
};
use serde_json::json;
use std::{
    env, fs,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

fn run(task: bool) {
    let model = env::var("PILL_LIVE_MODEL").expect("modelo explícito");
    let evidence = env::var("PILL_LIVE_EVIDENCE").expect("ruta evidencia");
    let python =
        PathBuf::from(env::var_os("PILL_TEST_PYTHON").expect("Python de pruebas explícito"));
    let sandbox = tempfile::tempdir().unwrap();
    let root = sandbox.path().join("Proyecto ñ");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("calc.py"), "def suma(a, b):\n    return a - b\n").unwrap();
    fs::write(root.join("test_calc.py"),"import calc\nassert calc.suma(2,3)==5, 'suma incorrecta'\nassert calc.suma(-2,3)==1\nprint('PRUEBA_SUMA_PASS')\n").unwrap();
    let mut config = Config::load(&root, None).unwrap().config;
    if let Ok(context) = env::var("PILL_LIVE_CONTEXT") {
        config.ollama.num_ctx = context.parse().expect("contexto explícito válido");
    }
    if let Ok(think) = env::var("PILL_LIVE_THINK") {
        config.ollama.think = think.parse().expect("thinking explícito válido");
    }
    if let Ok(predict) = env::var("PILL_LIVE_PREDICT") {
        config.ollama.num_predict = predict.parse().expect("límite explícito válido");
    }
    config.commands.commands = vec![CommandSpec {
        id: "test_suma".into(),
        program: python,
        args: vec!["-B".into(), "test_calc.py".into()],
        cwd: ".".into(),
        watched_files: vec!["test_calc.py".into()],
    }];
    let trust = sandbox.path().join("private/trust.json");
    let runner = CommandRunner::new(&root, config.commands.clone(), &trust).unwrap();
    let proposal = runner.proposals().unwrap().remove(0);
    // Consentimiento concreto del fixture de pruebas, nunca del modelo.
    runner.grant("test_suma", &proposal.fingerprint).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let before = rt
        .block_on(runner.execute("test_suma", Arc::new(AtomicBool::new(false))))
        .unwrap();
    assert_ne!(before.exit_code, Some(0));
    let mut worker = Worker::start_project(&root, &config, None, &trust).unwrap();
    fn next(worker: &mut Worker) -> Event {
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
            assert!(
                start.elapsed() < Duration::from_secs(240),
                "worker sin evento"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    match next(&mut worker) {
        Event::Notice { .. } => {}
        event => panic!("liberación de arranque: {event:?}"),
    }
    worker.submit(Action::Select(model.clone())).unwrap();
    match next(&mut worker) {
        Event::Selected(_) => {}
        event => panic!("selección: {event:?}"),
    };
    let prompt = "Corrige calc.py: suma debe sumar a y b. Lee calc.py con read_file, usa edit_file con su expected_sha256 real y cambia únicamente a - b por a + b. No edites test_calc.py. Ejecuta run_command con id test_suma y comprueba resultado. Reviewer debe leer calc.py antes de APPROVE.";
    worker
        .submit(if task {
            Action::Task(prompt.into())
        } else {
            Action::Chat(prompt.into())
        })
        .unwrap();
    let mut tools = Vec::new();
    let mut roles = Vec::new();
    let mut text = String::new();
    let mut terminal = String::new();
    let mut completed = false;
    let started = Instant::now();
    loop {
        match next(&mut worker) {
            Event::Delta(delta) => text.push_str(&delta),
            Event::Notice { role, text } => {
                roles.push(role);
                terminal = text;
            }
            Event::Tool {
                role,
                call_id,
                name,
                result,
                ..
            } => tools.push(json!({"role":role,"call_id":call_id,"name":name,"result":result})),
            Event::Complete { answer, .. } => {
                completed = true;
                if let Some(answer) = answer {
                    text = answer;
                }
                break;
            }
            Event::Failed(error) => {
                terminal = error;
                break;
            }
            event => panic!("evento inesperado: {event:?}"),
        }
    }
    drop(worker);
    let after = rt
        .block_on(runner.execute("test_suma", Arc::new(AtomicBool::new(false))))
        .unwrap();
    let content = fs::read_to_string(root.join("calc.py")).unwrap();
    let journals: Vec<_> = fs::read_dir(root.join(".pillcli/sessions"))
        .unwrap()
        .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();
    let success = completed
        && after.exit_code == Some(0)
        && tools.iter().any(|t| t["name"] == "read_file")
        && tools
            .iter()
            .any(|t| t["name"] == "edit_file" && t["result"]["status"] == "verified")
        && tools
            .iter()
            .any(|t| t["name"] == "run_command" && t["result"]["status"] == "verified")
        && (!task
            || [
                "coordinador",
                "explorador",
                "implementador",
                "tester",
                "reviewer",
            ]
            .iter()
            .all(|role| roles.iter().any(|r| r == role)));
    let report = json!({"result":if success{"PASS"}else{"FAIL"},"model":model,"task":task,"config":{"num_ctx":config.ollama.num_ctx,"num_predict":config.ollama.num_predict,"think":config.ollama.think},"before":before,"after":after,"content":content,"tools":tools,"roles":roles,"response":text,"last_notice_or_error":terminal,"elapsed_ms":started.elapsed().as_millis(),"journals":journals,"scope":"Fixture real descartable, inferencia local, herramientas y prueba antes/después; no modelos descargados ni efectos en archivos del usuario"});
    fs::write(evidence, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(success, "{}", report);
}

#[test]
#[ignore = "requiere Ollama y Python explícitos; edita solo fixture descartable"]
fn model_repairs_project_with_tools() {
    run(false);
}

#[test]
#[ignore = "requiere Ollama; cinco roles con conversaciones y herramientas reales"]
fn five_agents_repair_and_verify_project() {
    run(true);
}
