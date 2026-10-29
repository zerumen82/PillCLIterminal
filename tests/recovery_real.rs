#![cfg(windows)]

use std::{fs, io::Write};

use pill::{
    config::Config,
    files::Files,
    tools::recovery::{State, reconcile},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn files(path: &std::path::Path) -> Files {
    let config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    Files::new(path, config.files).unwrap()
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn requested(id: &str, tool: &str, path: Option<&str>) -> Value {
    json!({"session":"prueba", "event":format!("request-{id}"), "call_id":id,
        "stage":"requested", "tool":tool, "path":path})
}

fn observed(request: &Value, status: &str) -> Value {
    let mut value = request.clone();
    value["stage"] = json!("observed");
    value["event"] = json!(format!("observed-{}", request["call_id"]));
    value["status"] = json!(status);
    value
}

fn journal(files: &Files, name: &str, records: &[Value]) {
    let (_, _pins, mut file) = files.open_session_journal(name).unwrap();
    for record in records {
        serde_json::to_writer(&mut file, record).unwrap();
        file.write_all(b"\n").unwrap();
    }
    file.sync_all().unwrap();
}

fn recover(files: &Files) -> pill::tools::recovery::Report {
    reconcile(files, 16, 65_536, 16_384)
}

#[test]
fn empty_project_has_no_operations_to_replay() {
    let root = tempfile::tempdir().unwrap();
    let report = recover(&files(root.path()));
    assert!(!report.needs_review);
    assert!(report.entries.is_empty());
}

#[test]
fn lost_creation_response_is_reconciled_without_writing() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    fs::write(root.path().join("niño.txt"), "creado ñ\r\n").unwrap();
    let mut request = requested("1", "create_file", Some("niño.txt"));
    request["target_sha256"] = json!(digest("creado ñ\r\n"));
    journal(&files, "creacion.jsonl", &[request]);
    let bytes_before = fs::read(root.path().join("niño.txt")).unwrap();
    let report = recover(&files);
    assert!(!report.needs_review);
    assert_eq!(report.entries[0].state, State::Applied);
    assert_eq!(
        fs::read(root.path().join("niño.txt")).unwrap(),
        bytes_before
    );
    assert_eq!(files.session_count().unwrap(), 1);
}

#[test]
fn missing_creation_is_pending_and_is_not_created() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    journal(
        &files,
        "pendiente.jsonl",
        &[requested("1", "create_file", Some("ausente.txt"))],
    );
    let report = recover(&files);
    assert!(!report.needs_review);
    assert_eq!(report.entries[0].state, State::Pending);
    assert!(!root.path().join("ausente.txt").exists());
}

#[test]
fn intact_edit_base_is_pending_and_expected_target_is_applied() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    fs::write(root.path().join("base.txt"), "antes").unwrap();
    fs::write(root.path().join("aplicado.txt"), "después").unwrap();
    let mut base = requested("1", "edit_file", Some("base.txt"));
    base["expected_sha256"] = json!(digest("antes"));
    base["target_sha256"] = json!(digest("después"));
    let mut applied = base.clone();
    applied["call_id"] = json!("2");
    applied["path"] = json!("aplicado.txt");
    journal(&files, "edicion.jsonl", &[base, applied]);
    let report = recover(&files);
    assert!(!report.needs_review);
    assert_eq!(report.entries[0].state, State::Pending);
    assert_eq!(report.entries[1].state, State::Applied);
    assert_eq!(
        fs::read_to_string(root.path().join("base.txt")).unwrap(),
        "antes"
    );
}

#[test]
fn external_edit_remains_unknown_and_is_never_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    fs::write(root.path().join("usuario.txt"), "cambio externo").unwrap();
    let mut request = requested("1", "edit_file", Some("usuario.txt"));
    request["expected_sha256"] = json!(digest("base"));
    request["target_sha256"] = json!(digest("objetivo"));
    journal(&files, "conflicto.jsonl", &[request]);
    let report = recover(&files);
    assert!(report.needs_review);
    assert_eq!(report.entries[0].state, State::Unknown);
    assert_eq!(
        fs::read_to_string(root.path().join("usuario.txt")).unwrap(),
        "cambio externo"
    );
}

#[test]
fn incomplete_commands_and_mcp_are_unknown_after_restart() {
    let root = tempfile::tempdir().unwrap();
    {
        let files = files(root.path());
        journal(
            &files,
            "externos.jsonl",
            &[
                requested("1", "run_command", None),
                requested("2", "mcp__servidor__escribir", None),
            ],
        );
    }
    let reopened = files(root.path());
    let report = recover(&reopened);
    assert!(report.needs_review);
    assert_eq!(report.entries.len(), 2);
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.needs_review && entry.state == State::Unknown)
    );
    assert!(report.summary().contains("Efectos bloqueados"));
}

#[test]
fn durable_partial_is_not_cleared_by_matching_bytes_after_restart() {
    let root = tempfile::tempdir().unwrap();
    {
        let files = files(root.path());
        fs::write(root.path().join("parcial.txt"), "objetivo").unwrap();
        let request = requested("1", "edit_file", Some("parcial.txt"));
        let mut result = observed(&request, "partial");
        result["sha256"] = json!(digest("objetivo"));
        result["backup_path"] = json!("respaldo-conservado");
        journal(&files, "parcial.jsonl", &[request, result]);
    }
    let report = recover(&files(root.path()));
    assert!(report.needs_review);
    assert_eq!(
        report.entries[0].backup_path.as_deref(),
        Some("respaldo-conservado")
    );
}

#[test]
fn interrupted_batch_reports_each_file_and_preserves_completed_bytes() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    fs::write(root.path().join("uno.txt"), "uno").unwrap();
    fs::write(root.path().join("tres.txt"), "del usuario").unwrap();
    let first = requested("1", "create_file", Some("uno.txt"));
    let mut first_result = observed(&first, "verified");
    first_result["sha256"] = json!(digest("uno"));
    let second = requested("2", "create_file", Some("dos.txt"));
    let mut third = requested("3", "edit_file", Some("tres.txt"));
    third["expected_sha256"] = json!(digest("original"));
    journal(&files, "lote.jsonl", &[first, first_result, second, third]);
    let report = recover(&files);
    assert!(report.needs_review);
    assert_eq!(
        report
            .entries
            .iter()
            .map(|entry| entry.state)
            .collect::<Vec<_>>(),
        vec![State::Applied, State::Pending, State::Unknown]
    );
    assert_eq!(
        fs::read_to_string(root.path().join("uno.txt")).unwrap(),
        "uno"
    );
    assert!(!root.path().join("dos.txt").exists());
    assert_eq!(
        fs::read_to_string(root.path().join("tres.txt")).unwrap(),
        "del usuario"
    );
}

#[test]
fn truncated_or_oversized_record_blocks_effects() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    let (_, _pins, mut file) = files.open_session_journal("truncado.jsonl").unwrap();
    file.write_all(b"{\"session\":\"prueba\"").unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert!(recover(&files).needs_review);

    let other = tempfile::tempdir().unwrap();
    let files = self::files(other.path());
    journal(
        &files,
        "grande.jsonl",
        &[requested("1", "run_command", None)],
    );
    let report = reconcile(&files, 16, 65_536, 8);
    assert!(report.needs_review);
    assert!(report.entries[0].detail.contains("max_record_bytes"));
}

#[test]
fn duplicate_or_unpaired_effect_records_are_not_trusted() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    let request = requested("1", "create_file", Some("ausente.txt"));
    journal(&files, "duplicado.jsonl", &[request.clone(), request]);
    let orphan = observed(&requested("2", "run_command", None), "verified");
    journal(&files, "huerfano.jsonl", &[orphan]);
    let report = recover(&files);
    assert!(report.needs_review);
    assert_eq!(report.entries.len(), 2);
    assert!(report.entries.iter().all(|entry| entry.needs_review));
}

#[test]
fn verified_historical_write_does_not_overwrite_or_block_later_user_edit() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    fs::write(root.path().join("actual.txt"), "posterior del usuario").unwrap();
    let request = requested("1", "edit_file", Some("actual.txt"));
    let mut result = observed(&request, "verified");
    result["sha256"] = json!(digest("resultado antiguo"));
    journal(&files, "historico.jsonl", &[request, result]);
    let report = recover(&files);
    assert!(!report.needs_review);
    assert!(report.entries[0].detail.contains("versión actual difiere"));
    assert_eq!(
        fs::read_to_string(root.path().join("actual.txt")).unwrap(),
        "posterior del usuario"
    );
}

#[test]
fn move_and_delete_without_final_response_reconcile_actual_paths() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    fs::write(root.path().join("destino.txt"), "movido").unwrap();
    fs::write(root.path().join("por-mover.txt"), "base").unwrap();
    let mut moved = requested("1", "move_file", Some("origen.txt"));
    moved["destination"] = json!("destino.txt");
    moved["expected_sha256"] = json!(digest("movido"));
    let mut pending = requested("2", "move_file", Some("por-mover.txt"));
    pending["destination"] = json!("nuevo.txt");
    pending["expected_sha256"] = json!(digest("base"));
    let mut deleted = requested("3", "delete_file", Some("borrado.txt"));
    deleted["expected_sha256"] = json!(digest("eliminado"));
    journal(&files, "r09.jsonl", &[moved, pending, deleted]);
    let report = recover(&files);
    assert!(!report.needs_review);
    assert_eq!(
        report
            .entries
            .iter()
            .map(|entry| entry.state)
            .collect::<Vec<_>>(),
        vec![State::Applied, State::Pending, State::Applied]
    );
    assert_eq!(
        fs::read_to_string(root.path().join("por-mover.txt")).unwrap(),
        "base"
    );
    assert!(!root.path().join("nuevo.txt").exists());
}

#[test]
fn journal_path_cannot_escape_the_authorized_root() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    let mut request = requested("1", "edit_file", Some("../fuera.txt"));
    request["expected_sha256"] = json!(digest("base"));
    journal(&files, "ruta.jsonl", &[request]);
    let report = recover(&files);
    assert!(report.needs_review);
    assert!(report.entries[0].detail.contains("no se pudo inspeccionar"));
}

#[test]
fn incomplete_metadata_and_delete_without_base_remain_unknown() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    journal(
        &files,
        "metadatos.jsonl",
        &[
            json!({"session":"prueba", "stage":"etapa_desconocida", "tool":"delete_file"}),
            requested("2", "delete_file", Some("ausente.txt")),
        ],
    );
    let report = recover(&files);
    assert!(report.needs_review);
    assert_eq!(report.entries.len(), 2);
    assert!(report.entries.iter().all(|entry| entry.needs_review));
}

#[test]
fn awaiting_confirmation_did_not_execute_and_summary_does_not_emit_journal_text() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    let request = requested("1", "delete_file", Some("sin-conceder.txt"));
    let result = observed(&request, "awaiting_confirmation");
    journal(&files, "permiso.jsonl", &[request, result]);
    let report = recover(&files);
    assert!(!report.needs_review);
    assert_eq!(report.entries[0].state, State::Pending);

    journal(
        &files,
        "datos.jsonl",
        &[requested(
            "2",
            "mcp__servidor__tool",
            Some("\u{1b}[31mINJECTAR"),
        )],
    );
    let report = recover(&files);
    assert!(report.needs_review);
    assert!(!report.summary().contains("INJECTAR"));
    assert!(!report.summary().contains('\u{1b}'));
}

#[test]
fn historical_mcp_failures_and_interrupted_commands_are_not_safe_to_retry() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    let mcp = requested("1", "mcp__servidor__tool", None);
    let mcp_failed = observed(&mcp, "failed");
    let command = requested("2", "run_command", None);
    let mut timed_out = observed(&command, "failed");
    timed_out["command"] = json!({"timed_out":true, "cancelled":false});
    let cancelled_command = requested("3", "run_command", None);
    let mut cancelled = observed(&cancelled_command, "failed");
    cancelled["command"] = json!({"timed_out":false, "cancelled":true});
    let rejected = requested("4", "mcp__servidor__tool", None);
    let mut before_send = observed(&rejected, "failed");
    before_send["effect_not_started"] = json!(true);
    journal(
        &files,
        "compatibilidad.jsonl",
        &[
            mcp,
            mcp_failed,
            command,
            timed_out,
            cancelled_command,
            cancelled,
            rejected,
            before_send,
        ],
    );
    let report = recover(&files);
    assert!(report.needs_review);
    assert_eq!(
        report
            .entries
            .iter()
            .map(|entry| entry.state)
            .collect::<Vec<_>>(),
        vec![
            State::Unknown,
            State::Unknown,
            State::Unknown,
            State::Pending
        ]
    );
}
