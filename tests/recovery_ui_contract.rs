#![cfg(windows)]
use pill::{
    config::Config,
    providers::ollama::{ToolCall, ToolFunction},
    tools::{Role, Toolset},
};
use serde_json::json;
use std::sync::{Arc, atomic::AtomicBool};
#[test]
fn demonstrated_recovery_unblocks_the_same_executor_but_unknown_commands_keep_the_gate() {
    let sandbox = tempfile::tempdir().unwrap();
    let root = sandbox.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    let tools = Toolset::new(
        &root,
        &config,
        None,
        &sandbox.path().join("private/trust.json"),
    )
    .unwrap();
    let call = |name: &str, args| ToolCall {
        function: ToolFunction {
            name: name.into(),
            arguments: args,
        },
    };
    let create = call("create_file", json!({"path":"a.txt","content":"data"}));
    let mut journal = tools.journal().unwrap();
    journal
        .requested(
            Role::Implementer,
            &create,
            tools.intended_sha256(&create).as_deref(),
        )
        .unwrap();
    std::fs::write(root.join("a.txt"), b"data").unwrap();
    drop(journal);
    tools.block_effects();
    assert_eq!(tools.recovery_view().unwrap()["needs_review"], false);
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    assert_eq!(
        rt.block_on(tools.execute(
            Role::Implementer,
            &call("create_file", json!({"path":"b.txt","content":"ok"})),
            Arc::new(AtomicBool::new(false))
        ))["status"],
        "verified"
    );
    let mut journal = tools.journal().unwrap();
    journal
        .requested(
            Role::Implementer,
            &call("run_command", json!({"id":"unknown"})),
            None,
        )
        .unwrap();
    drop(journal);
    assert_eq!(tools.recovery_view().unwrap()["needs_review"], true);
    assert_eq!(
        rt.block_on(tools.execute(
            Role::Implementer,
            &call("create_file", json!({"path":"c.txt","content":"blocked"})),
            Arc::new(AtomicBool::new(false))
        ))["status"],
        "failed"
    );
    assert!(!root.join("c.txt").exists());
}

#[test]
fn multi_edit_intention_matches_simultaneous_publication_including_bom() {
    let sandbox = tempfile::tempdir().unwrap();
    let root = sandbox.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    let tools = Toolset::new(
        &root,
        &config,
        None,
        &sandbox.path().join("private/trust.json"),
    )
    .unwrap();
    std::fs::write(root.join("a.txt"), b"\xef\xbb\xbfA B\r\n").unwrap();
    let call = |name: &str, args| ToolCall {
        function: ToolFunction {
            name: name.into(),
            arguments: args,
        },
    };
    let read = tools.execute_reads(
        Role::Explorer,
        &[call("read_file", json!({"path":"a.txt"}))],
        Arc::new(AtomicBool::new(false)),
    );
    let edit = call(
        "edit_file_many",
        json!({"path":"a.txt","expected_sha256":read[0]["sha256"],"edits":[{"old":"A","new":"B"},{"old":"B","new":"C"}]}),
    );
    let intended = tools.intended_sha256(&edit).unwrap();
    let mut journal = tools.journal().unwrap();
    journal
        .requested(Role::Implementer, &edit, Some(&intended))
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let result =
        rt.block_on(tools.execute(Role::Implementer, &edit, Arc::new(AtomicBool::new(false))));
    drop(journal);
    assert_eq!(result["status"], "verified");
    assert_eq!(result["sha256"], intended);
    assert_eq!(
        std::fs::read(root.join("a.txt")).unwrap(),
        b"\xef\xbb\xbfB C\r\n"
    );
    let recovery = tools.recovery_view().unwrap();
    assert_eq!(recovery["entries"][0]["state"], "applied");
    assert_eq!(recovery["needs_review"], false);
}
