#![cfg(windows)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use pill::{
    config::{Config, McpServerConfig},
    providers::ollama::{ToolCall, ToolFunction},
    tools::{Role, Toolset, commands::CommandSpec},
};
use serde_json::{Value, json};
use tempfile::TempDir;

fn python_program() -> PathBuf {
    if let Some(value) = std::env::var_os("PILL_TEST_PYTHON") {
        return PathBuf::from(value)
            .canonicalize()
            .expect("PILL_TEST_PYTHON debe ser un ejecutable");
    }
    std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
        .map(|directory| directory.join("python.exe"))
        .find(|path| path.is_file())
        .expect("python.exe en PATH o PILL_TEST_PYTHON")
        .canonicalize()
        .unwrap()
}

fn powershell() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
        .map(|directory| directory.join("pwsh.exe"))
        .find(|path| path.is_file())
        .expect("pwsh.exe")
        .canonicalize()
        .unwrap()
}

struct Project {
    _temporary: TempDir,
    root: PathBuf,
    trust: PathBuf,
}

impl Project {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("proyecto");
        fs::create_dir(&root).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("diagnostics/mcp_fixture.py");
        fs::copy(source, root.join("mcp_fixture.py")).unwrap();
        fs::write(root.join("eco.ps1"), "exit 0").unwrap();
        let trust = temporary.path().join("privado").join("trust.json");
        Self {
            _temporary: temporary,
            root,
            trust,
        }
    }
}

fn config(_project: &Project, mode: &str, startup_ms: u64, call_ms: u64) -> Config {
    let mut config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    config.gpu.exclusive = true;
    config.files.extra_roots = vec![PathBuf::from(r"C:\Windows")];
    config.commands.commands = vec![CommandSpec {
        id: "eco".into(),
        program: powershell(),
        args: vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-File".into(),
            "eco.ps1".into(),
        ],
        cwd: PathBuf::from("."),
        watched_files: vec!["eco.ps1".into()],
    }];
    config.mcp.startup_timeout_ms = startup_ms;
    config.mcp.call_timeout_ms = call_ms;
    config.mcp.servers = vec![McpServerConfig {
        id: "eco".into(),
        program: python_program(),
        args: vec!["mcp_fixture.py".into(), mode.into()],
        cwd: PathBuf::from("."),
        env: Vec::new(),
    }];
    config
}

fn toolset(project: &Project, mode: &str, startup_ms: u64, call_ms: u64) -> Toolset {
    Toolset::new(
        &project.root,
        &config(project, mode, startup_ms, call_ms),
        None,
        &project.trust,
    )
    .unwrap()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        function: ToolFunction {
            name: name.into(),
            arguments,
        },
    }
}

fn offer<'a>(offers: &'a [pill::tools::mcp::Offer], id: &str) -> &'a pill::tools::mcp::Offer {
    offers.iter().find(|offer| offer.id == id).expect(id)
}

#[test]
fn one_grant_does_not_approve_the_others_and_echo_is_a_real_process() {
    let project = Project::new();
    let tools = toolset(&project, "echo", 5_000, 30_000);
    let offers = tools.offers().unwrap();
    assert!(offers.iter().any(|offer| offer.id == "command:eco"));
    assert!(offers.iter().any(|offer| offer.id == "mcp:eco"));
    assert!(offers.iter().any(|offer| offer.id == "root:0"));
    assert!(offers.iter().any(|offer| offer.id == "exclusive"));
    let root = offer(&offers, "root:0");
    assert_eq!(
        tools.grant_offer(&root.id, &root.fingerprint).unwrap_err(),
        "una raíz extra no amplía el alcance"
    );
    let command = offer(&offers, "command:eco");
    tools
        .grant_offer(&command.id, &command.fingerprint)
        .unwrap();
    assert!(!project.root.join("started.txt").exists());
    let denied = runtime().block_on(tools.execute(
        Role::Implementer,
        &call("mcp__eco__echo", json!({"text": "hola ñ"})),
        Arc::new(AtomicBool::new(false)),
    ));
    assert_ne!(denied["status"], "verified");
    assert!(!project.root.join("started.txt").exists());
    let fresh = tools.offers().unwrap();
    let mcp = offer(&fresh, "mcp:eco");
    assert!(mcp.detail.contains("\"granted\":false"));
    tools.grant_offer(&mcp.id, &mcp.fingerprint).unwrap();
    assert!(!tools.exclusive_granted().unwrap());
    let result = runtime().block_on(tools.execute(
        Role::Implementer,
        &call("mcp__eco__echo", json!({"text": "hola ñ"})),
        Arc::new(AtomicBool::new(false)),
    ));
    assert_eq!(
        result["status"],
        "verified",
        "resultado={result} inbox={}",
        fs::read_to_string(project.root.join("inbox.txt")).unwrap_or_default()
    );
    assert_eq!(result["text"], "hola ñ");
    assert_eq!(
        fs::read_to_string(project.root.join("started.txt")).unwrap(),
        "echo"
    );
    let explorer = runtime().block_on(tools.execute(
        Role::Explorer,
        &call("mcp__eco__echo", json!({"text": "no"})),
        Arc::new(AtomicBool::new(false)),
    ));
    assert_eq!(explorer["status"], "failed");
    fs::write(
        project.root.join("mcp_fixture.py"),
        fs::read_to_string(project.root.join("mcp_fixture.py")).unwrap() + "\n# cambiado\n",
    )
    .unwrap();
    let stale = runtime().block_on(tools.execute(
        Role::Tester,
        &call("mcp__eco__echo", json!({"text": "otra"})),
        Arc::new(AtomicBool::new(false)),
    ));
    assert_ne!(stale["status"], "verified");
    assert!(
        stale.to_string().contains("conceder") || stale["error"].to_string().contains("conceder")
    );
}

#[test]
fn external_schema_refs_fail_without_publishing_a_tool() {
    let project = Project::new();
    for mode in ["badref", "badfile"] {
        let tools = toolset(&project, mode, 5_000, 30_000);
        let fresh = tools.offers().unwrap();
        let mcp = offer(&fresh, "mcp:eco");
        tools.grant_offer(&mcp.id, &mcp.fingerprint).unwrap();
        let (schemas, errors) = runtime().block_on(tools.mcp_schemas());
        assert!(schemas.is_empty(), "{mode}: {schemas:?}");
        let error = errors.join("\n");
        assert!(
            error.contains("referencia externa"),
            "{mode}: {error} inbox={}",
            fs::read_to_string(project.root.join("inbox.txt")).unwrap_or_default()
        );
        assert!(!error.contains("[fonts]"));
        assert!(!error.contains("example.invalid/no-existe.json contents"));
    }
}

#[test]
fn a_server_that_hangs_or_dies_is_not_success() {
    let project = Project::new();
    let hang = toolset(&project, "hang", 500, 800);
    let fresh_hang = hang.offers().unwrap();
    let offer_hang = offer(&fresh_hang, "mcp:eco");
    hang.grant_offer(&offer_hang.id, &offer_hang.fingerprint)
        .unwrap();
    let started = std::time::Instant::now();
    let (schemas, errors) = runtime().block_on(hang.mcp_schemas());
    assert!(started.elapsed().as_secs() < 10);
    assert!(schemas.is_empty());
    assert!(
        errors
            .iter()
            .any(|error| error.contains("no respondió") || error.contains("no es un éxito")),
        "{errors:?}"
    );

    let die = toolset(&project, "die", 5_000, 5_000);
    let fresh_die = die.offers().unwrap();
    let offer_die = offer(&fresh_die, "mcp:eco");
    die.grant_offer(&offer_die.id, &offer_die.fingerprint)
        .unwrap();
    let result = runtime().block_on(die.execute(
        Role::Implementer,
        &call("mcp__eco__echo", json!({"text": "corte"})),
        Arc::new(AtomicBool::new(false)),
    ));
    assert_ne!(result["status"], "verified", "{result}");
}

#[test]
fn an_external_file_argument_is_rejected_before_launch() {
    let project = Project::new();
    let mut config = config(&project, "echo", 5_000, 30_000);
    config.mcp.servers[0]
        .args
        .push(r"C:\Windows\win.ini".into());
    let tools = Toolset::new(&project.root, &config, None, &project.trust);
    let error = tools.unwrap().offers().unwrap_err();
    assert!(error.contains("fuera de la raíz"), "{error}");
    assert!(!project.root.join("started.txt").exists());
}

#[test]
fn effects_before_death_timeout_or_tool_error_block_following_effects_and_restart() {
    for mode in ["write_die", "write_hang", "write_error"] {
        let project = Project::new();
        let config = config(&project, mode, 1000, 1000);
        let tools = Toolset::new(&project.root, &config, None, &project.trust).unwrap();
        let offer = tools
            .offers()
            .unwrap()
            .into_iter()
            .find(|o| o.id == "mcp:eco")
            .unwrap();
        tools.grant_offer(&offer.id, &offer.fingerprint).unwrap();
        let request = call("mcp__eco__echo", json!({"text":"real"}));
        let mut journal = tools.journal().unwrap();
        let id = journal
            .record(Role::Implementer, "requested", Some(&request), None)
            .unwrap();
        let result = runtime().block_on(tools.execute(
            Role::Implementer,
            &request,
            Arc::new(AtomicBool::new(false)),
        ));
        journal
            .observed(Role::Implementer, &request, &result, &id)
            .unwrap();
        assert_eq!(result["status"], "unknown", "{mode}: {result}");
        assert_eq!(
            fs::read_to_string(project.root.join("effect.txt")).unwrap(),
            "efecto real antes del resultado"
        );
        let create = call(
            "create_file",
            json!({"path":"no.txt","content":"bloqueado"}),
        );
        let rejected = runtime().block_on(tools.execute(
            Role::Implementer,
            &create,
            Arc::new(AtomicBool::new(false)),
        ));
        assert_eq!(rejected["status"], "failed");
        let restored = Toolset::new(&project.root, &config, None, &project.trust).unwrap();
        assert!(restored.recovery_report().contains("Efectos bloqueados"));
        let rejected = runtime().block_on(restored.execute(
            Role::Implementer,
            &create,
            Arc::new(AtomicBool::new(false)),
        ));
        assert_eq!(rejected["status"], "failed");
        assert!(!project.root.join("no.txt").exists());
    }
}

#[test]
fn oversized_arguments_are_rejected_before_starting_server_and_ambiguous_ids_are_invalid() {
    let project = Project::new();
    let mut config = config(&project, "echo", 1000, 1000);
    config.mcp.max_message_bytes = 1024;
    let tools = Toolset::new(&project.root, &config, None, &project.trust).unwrap();
    let offers = tools.offers().unwrap();
    let offer = offer(&offers, "mcp:eco");
    tools.grant_offer(&offer.id, &offer.fingerprint).unwrap();
    let result = runtime().block_on(tools.execute(
        Role::Implementer,
        &call("mcp__eco__echo", json!({"text":"a".repeat(2048)})),
        Arc::new(AtomicBool::new(false)),
    ));
    assert_eq!(result["status"], "failed");
    assert!(!project.root.join("started.txt").exists());
    config.mcp.servers[0].id = "eco__otra".into();
    assert!(config.mcp.validate().is_err());
}

#[test]
fn catalog_cache_reuses_metadata_but_rechecks_revocation_and_script_hash() {
    let project = Project::new();
    let tools = toolset(&project, "echo", 5_000, 5_000);
    let offers = tools.offers().unwrap();
    let mcp = offer(&offers, "mcp:eco");
    tools.grant_offer(&mcp.id, &mcp.fingerprint).unwrap();
    let rt = runtime();
    let first = rt.block_on(tools.mcp_schemas());
    assert!(first.1.is_empty(), "{:?}", first.1);
    assert!(!first.0.is_empty());
    let launches = fs::read(project.root.join("launches.txt")).unwrap();
    let second = rt.block_on(tools.mcp_schemas());
    assert_eq!(first.0, second.0);
    assert_eq!(
        launches,
        fs::read(project.root.join("launches.txt")).unwrap()
    );
    tools.revoke_offer("mcp:eco").unwrap();
    assert!(rt.block_on(tools.mcp_schemas()).0.is_empty());
    assert_eq!(
        launches,
        fs::read(project.root.join("launches.txt")).unwrap()
    );
    let offers = tools.offers().unwrap();
    let mcp = offer(&offers, "mcp:eco");
    tools.grant_offer(&mcp.id, &mcp.fingerprint).unwrap();
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(project.root.join("mcp_fixture.py"))
        .unwrap()
        .write_all(b"\n# script changed after cached catalog\n")
        .unwrap();
    assert!(rt.block_on(tools.mcp_schemas()).0.is_empty());
    assert_eq!(
        launches,
        fs::read(project.root.join("launches.txt")).unwrap()
    );
}
#[test]
fn shutting_down_own_mcp_also_finishes_its_remaining_descendant() {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    let project = Project::new();
    let tools = toolset(&project, "descendant", 5_000, 5_000);
    let offers = tools.offers().unwrap();
    let mcp = offer(&offers, "mcp:eco");
    tools.grant_offer(&mcp.id, &mcp.fingerprint).unwrap();
    let result = runtime().block_on(tools.mcp_schemas());
    assert!(result.1.is_empty(), "{:?}", result.1);
    assert!(!result.0.is_empty());
    let pid = fs::read_to_string(project.root.join("descendant.txt"))
        .unwrap()
        .parse::<u32>()
        .unwrap();
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !handle.is_null() {
            let state = WaitForSingleObject(handle, 0);
            CloseHandle(handle);
            assert_eq!(state, WAIT_OBJECT_0, "el descendiente propio sigue activo");
        }
    }
}
