#![cfg(windows)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use pill::{
    config::{Config, McpServerConfig},
    providers::ollama::{ToolCall, ToolFunction},
    tools::{Role, Toolset, commands::CommandSpec, schemas},
};
use serde_json::{Value, json};

fn executable(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
        .expect("ejecutable instalado para la prueba")
        .canonicalize()
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

#[test]
fn readonly_role_cannot_use_effects_even_when_the_user_granted_their_definitions() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("Proyecto solo lectura ñ");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("base.txt"), "original\r\n").unwrap();
    fs::write(
        root.join("probe.ps1"),
        "Set-Content -LiteralPath 'command-effect.txt' -Value 'ejecutado'; exit 0",
    )
    .unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("diagnostics/mcp_fixture.py"),
        root.join("mcp_fixture.py"),
    )
    .unwrap();
    let mut config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    config.commands.commands = vec![CommandSpec {
        id: "probe".into(),
        program: executable("pwsh.exe"),
        args: vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-File".into(),
            "probe.ps1".into(),
        ],
        cwd: ".".into(),
        watched_files: vec!["probe.ps1".into()],
    }];
    config.mcp.servers = vec![McpServerConfig {
        id: "probe".into(),
        program: std::env::var_os("PILL_TEST_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| executable("python.exe")),
        args: vec!["mcp_fixture.py".into(), "echo".into()],
        cwd: ".".into(),
        env: Vec::new(),
    }];
    let tools = Toolset::new(
        &root,
        &config,
        None,
        &temporary.path().join("private/trust.json"),
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for offer in tools.offers().unwrap() {
        if offer.id.starts_with("command:") || offer.id.starts_with("mcp:") {
            // Consentimiento humano concreto del fixture, nunca del modelo.
            tools.grant_offer(&offer.id, &offer.fingerprint).unwrap();
        }
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let read = runtime.block_on(tools.execute(
        Role::Explorer,
        &call("read_file", json!({"path":"base.txt"})),
        cancel.clone(),
    ));
    assert_eq!(read["status"], "verified");
    let sha = read["sha256"].as_str().unwrap();
    let requests = [
        call(
            "create_file",
            json!({"path":"created.txt","content":"no autorizado"}),
        ),
        call(
            "edit_file",
            json!({"path":"base.txt","expected_sha256":sha,"old":"original","new":"alterado"}),
        ),
        call(
            "edit_file_many",
            json!({"path":"base.txt","expected_sha256":sha,"edits":[{"old":"original","new":"alterado"}]}),
        ),
        call(
            "move_file",
            json!({"path":"base.txt","destination":"moved.txt","expected_sha256":sha}),
        ),
        call(
            "delete_file",
            json!({"path":"base.txt","expected_sha256":sha}),
        ),
        call("run_command", json!({"id":"probe"})),
        call("mcp__probe__echo", json!({"text":"no arrancar"})),
    ];
    for request in requests {
        let result = runtime.block_on(tools.execute(Role::Explorer, &request, cancel.clone()));
        assert_eq!(
            result["status"], "failed",
            "{}: {result}",
            request.function.name
        );
        assert!(
            result["error"].as_str().unwrap().contains("rol"),
            "{result}"
        );
    }
    assert_eq!(fs::read(root.join("base.txt")).unwrap(), b"original\r\n");
    for name in [
        "created.txt",
        "moved.txt",
        "command-effect.txt",
        "started.txt",
        "inbox.txt",
    ] {
        assert!(!root.join(name).exists(), "efecto indebido: {name}");
    }
    assert!(
        !tools
            .offers()
            .unwrap()
            .iter()
            .any(|offer| offer.id.starts_with("file:"))
    );
    let names: Vec<String> = schemas(Role::Explorer, true)
        .into_iter()
        .map(|schema| schema["function"]["name"].as_str().unwrap().to_owned())
        .collect();
    assert!(names.contains(&"read_file".into()));
    assert!(names.contains(&"inspect_path".into()));
    assert!(!names.iter().any(|name| name == "run_command"
        || name.starts_with("mcp__")
        || name.contains("edit")
        || name.contains("create")
        || name.contains("delete")
        || name.contains("move")));
}

#[test]
fn rejected_readonly_operation_does_not_consume_a_concrete_human_file_grant() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("base.txt"), "base").unwrap();
    let config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    let tools = Toolset::new(
        &root,
        &config,
        None,
        &temporary.path().join("private/trust.json"),
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let read = runtime.block_on(tools.execute(
        Role::Implementer,
        &call("read_file", json!({"path":"base.txt"})),
        cancel.clone(),
    ));
    let request = call(
        "delete_file",
        json!({"path":"base.txt","expected_sha256":read["sha256"]}),
    );
    let proposed = runtime.block_on(tools.execute(Role::Implementer, &request, cancel.clone()));
    assert_eq!(proposed["status"], "awaiting_confirmation");
    let offer = tools
        .offers()
        .unwrap()
        .into_iter()
        .find(|offer| Some(offer.id.as_str()) == proposed["offer"].as_str())
        .unwrap();
    tools.grant_offer(&offer.id, &offer.fingerprint).unwrap();
    let denied = runtime.block_on(tools.execute(Role::Explorer, &request, cancel.clone()));
    assert_eq!(denied["status"], "failed");
    assert_eq!(fs::read_to_string(root.join("base.txt")).unwrap(), "base");
    let applied = runtime.block_on(tools.execute(Role::Implementer, &request, cancel));
    assert_eq!(applied["status"], "verified", "{applied}");
    assert!(!root.join("base.txt").exists());
}
