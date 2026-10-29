#![cfg(windows)]

use pill::{
    config::Config,
    providers::ollama::{ToolCall, ToolFunction},
    tools::{Role, Toolset},
};
use serde_json::{Value, json};
use std::{
    fs,
    sync::{Arc, atomic::AtomicBool},
};

fn call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        function: ToolFunction {
            name: name.into(),
            arguments: args,
        },
    }
}

#[test]
fn independent_read_batch_preserves_order_versions_and_permission_boundaries() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("proyecto ñ");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("uno.txt"), "primero").unwrap();
    fs::write(root.join("dos.txt"), "segundo").unwrap();
    let config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    let tools = Toolset::new(
        &root,
        &config,
        None,
        &temporary.path().join("private/trust.json"),
    )
    .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let calls = [
        call("read_file", json!({"path":"uno.txt"})),
        call("read_file", json!({"path":"dos.txt"})),
    ];
    let results = tools.execute_reads(Role::Explorer, &calls, cancel.clone());
    assert_eq!(results[0]["content"], "primero");
    assert_eq!(results[1]["content"], "segundo");
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    for (path, old, result) in [
        ("uno.txt", "primero", &results[0]),
        ("dos.txt", "segundo", &results[1]),
    ] {
        let edit = call(
            "edit_file",
            json!({"path":path,"expected_sha256":result["sha256"],"old":old,"new":"observado"}),
        );
        assert_eq!(
            rt.block_on(tools.execute(Role::Implementer, &edit, cancel.clone()))["status"],
            "verified"
        );
        assert_eq!(fs::read_to_string(root.join(path)).unwrap(), "observado");
    }
    let forbidden = tools.execute_reads(
        Role::Implementer,
        &[call("create_file", json!({"path":"no.txt","content":"no"}))],
        cancel,
    );
    assert_eq!(forbidden[0]["status"], "failed");
    assert!(!root.join("no.txt").exists());
    tools.verify_versions().unwrap();
}

#[test]
fn a_batch_reserves_each_durable_result_before_starting_any_effect() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("project");
    fs::create_dir(&root).unwrap();
    let mut config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    config.agents.max_record_bytes = 4096;
    config.agents.max_journal_bytes = 8192;
    let tools = Toolset::new(
        &root,
        &config,
        None,
        &temporary.path().join("private/trust.json"),
    )
    .unwrap();
    let mut journal = tools.journal().unwrap();
    let first = call("create_file", json!({"path":"uno.txt","content":"uno"}));
    let second = call("create_file", json!({"path":"dos.txt","content":"dos"}));
    journal.requested(Role::Implementer, &first, None).unwrap();
    assert!(journal.requested(Role::Implementer, &second, None).is_err());
    assert!(!root.join("uno.txt").exists());
    assert!(!root.join("dos.txt").exists());
    let bytes = fs::read_to_string(&journal.path).unwrap();
    assert_eq!(bytes.lines().count(), 1);
}

#[test]
fn two_large_real_reads_overlap_on_independent_threads() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("project");
    fs::create_dir(&root).unwrap();
    let bytes = vec![b'x'; 8 * 1024 * 1024];
    fs::write(root.join("uno.txt"), &bytes).unwrap();
    fs::write(root.join("dos.txt"), &bytes).unwrap();
    let mut config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    config.files.max_file_bytes = bytes.len();
    let tools = Toolset::new(
        &root,
        &config,
        None,
        &temporary.path().join("private/trust.json"),
    )
    .unwrap();
    let calls = [
        call("read_file", json!({"path":"uno.txt"})),
        call("read_file", json!({"path":"dos.txt"})),
    ];
    let results =
        tools.execute_reads_traced(Role::Explorer, &calls, Arc::new(AtomicBool::new(false)));
    let mut evidence = Vec::new();
    for (value, execution) in &results {
        assert_eq!(value["status"], "verified");
        assert_eq!(value["content"].as_str().unwrap().as_bytes(), bytes);
        evidence.push(json!({"path":value["path"],"sha256":value["sha256"],"bytes":value["bytes"],"execution":execution}));
    }
    assert_ne!(results[0].1["thread"], results[1].1["thread"]);
    let latest_start = results
        .iter()
        .map(|(_, e)| e["started_ns"].as_u64().unwrap())
        .max()
        .unwrap();
    let earliest_finish = results
        .iter()
        .map(|(_, e)| e["finished_ns"].as_u64().unwrap())
        .min()
        .unwrap();
    let overlap = latest_start < earliest_finish;
    if let Some(path) = std::env::var_os("PILL_PARALLEL_EVIDENCE") {
        fs::write(path,serde_json::to_vec_pretty(&json!({"result":if overlap {"PASS"} else {"FAIL"},"reads":evidence,"overlap_ns":earliest_finish.saturating_sub(latest_start),"scope":"Dos lecturas reales, sin barrera ni pausas artificiales; tiempos tomados dentro de cada hilo"})).unwrap()).unwrap();
    }
    assert!(overlap, "las lecturas no se solaparon: {evidence:?}");
}

#[test]
fn repeated_batches_reuse_the_configured_worker_instead_of_spawning_per_read() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"real").unwrap();
    let mut config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    config.agents.max_parallel_reads = 1;
    let trust = tempfile::tempdir().unwrap();
    let tools = Toolset::new(root.path(), &config, None, &trust.path().join("trust.json")).unwrap();
    let mut threads = std::collections::BTreeSet::new();
    for _ in 0..4 {
        let result = tools.execute_reads_traced(
            Role::Explorer,
            &[call("read_file", json!({"path":"a.txt"}))],
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(result[0].0["content"], "real");
        threads.insert(result[0].1["thread"].as_str().unwrap().to_owned());
    }
    assert_eq!(threads.len(), 1);
}
