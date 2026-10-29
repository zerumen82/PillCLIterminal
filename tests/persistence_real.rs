#![cfg(windows)]

use std::{fs, io::Write, path::Path};

use pill::{
    config::Config,
    files::{ExactEdit, Files},
    tools::recovery::{self, State},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn files(root: &Path) -> Files {
    let config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    Files::new(root, config.files).unwrap()
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn transcript(text: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"schema_version":2,"model":"m","digest":"d","dropped":0,
        "messages":[{"role":"system","content":"sistema"},{"role":"user","content":"petición"},{"role":"assistant","content":text}]})).unwrap()
}

fn journal(files: &Files, name: &str, records: &[Value]) {
    let (_, _pins, mut file) = files.open_session_journal(name).unwrap();
    for record in records {
        serde_json::to_writer(&mut file, record).unwrap();
        file.write_all(b"\n").unwrap();
    }
    file.sync_all().unwrap();
}

fn unknown_request(path: &str) -> Value {
    json!({"session":"real","event":"request","call_id":"one","stage":"requested",
        "tool":"mcp__server__tool","path":path})
}

#[test]
fn private_publication_keeps_a_valid_base_and_recovers_corruption_or_missing_primary() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    let first = transcript("primera");
    let second = transcript("segunda");
    files.write_private("model.json", &first).unwrap();
    files.write_private("model.json", &second).unwrap();
    assert_eq!(files.read_private("model.json").unwrap().unwrap(), second);
    fs::write(root.path().join(".pillcli/model.json"), b"{truncated").unwrap();
    let recovered = files.read_private_checked("model.json").unwrap();
    assert!(recovered.recovered);
    assert!(recovered.warning.is_some());
    assert_eq!(recovered.bytes.unwrap(), first);
    // La muestra representa la ventana tras mover la base; no se repite la escritura.
    fs::remove_file(root.path().join(".pillcli/model.json")).unwrap();
    let recovered = files.read_private_checked("model.json").unwrap();
    assert!(recovered.recovered);
    assert_eq!(recovered.bytes.unwrap(), first);
    assert!(!root.path().join(".pillcli/model.json").exists());
}

#[test]
fn a_private_write_failure_preserves_current_bytes_and_the_previous_base() {
    let root = tempfile::tempdir().unwrap();
    let mut config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    config.files.max_transactions = 2;
    let files = Files::new(root.path(), config.files).unwrap();
    files.write_private("model.json", &transcript("a")).unwrap();
    files.write_private("model.json", &transcript("b")).unwrap();
    let current = fs::read(root.path().join(".pillcli/model.json")).unwrap();
    assert!(files.write_private("model.json", &transcript("c")).is_err());
    assert_eq!(
        fs::read(root.path().join(".pillcli/model.json")).unwrap(),
        current
    );
    assert_eq!(files.retention_status().unwrap()["private_versions"], 2);
    assert!(files.write_private("model.json", b"not json").is_err());
    assert_eq!(
        fs::read(root.path().join(".pillcli/model.json")).unwrap(),
        current
    );
}

#[test]
fn multiple_exact_edits_publish_once_preserve_bom_crlf_and_reject_partial_batches() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    let original = b"\xef\xbb\xbfuno = 1\r\ndos = 2\r\n";
    fs::write(root.path().join("código.txt"), original).unwrap();
    let sha = hash(original);
    let invalid = [
        ExactEdit {
            old: "uno = 1".into(),
            new: "uno = 10".into(),
        },
        ExactEdit {
            old: "no coincide".into(),
            new: "sin efecto".into(),
        },
    ];
    assert!(files.edit_many("código.txt", &sha, &invalid).is_err());
    assert_eq!(fs::read(root.path().join("código.txt")).unwrap(), original);
    assert_eq!(files.retention_status().unwrap()["root_backups"], 0);
    let valid = [
        ExactEdit {
            old: "uno = 1".into(),
            new: "uno = 10".into(),
        },
        ExactEdit {
            old: "dos = 2".into(),
            new: "dos = 20".into(),
        },
    ];
    let result = files.edit_many("código.txt", &sha, &valid).unwrap();
    assert_eq!(result["status"], "verified");
    assert_eq!(
        fs::read(root.path().join("código.txt")).unwrap(),
        b"\xef\xbb\xbfuno = 10\r\ndos = 20\r\n"
    );
    assert_eq!(
        fs::read(result["backup_path"].as_str().unwrap()).unwrap(),
        original
    );
    assert_eq!(files.retention_status().unwrap()["root_backups"], 1);
}

#[test]
fn overlapping_or_repeated_edits_do_not_modify_the_file() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    fs::write(root.path().join("a.txt"), "abcdef").unwrap();
    for edits in [
        vec![
            ExactEdit {
                old: "abc".into(),
                new: "a".into(),
            },
            ExactEdit {
                old: "bc".into(),
                new: "b".into(),
            },
        ],
        vec![
            ExactEdit {
                old: "abc".into(),
                new: "a".into(),
            },
            ExactEdit {
                old: "abc".into(),
                new: "b".into(),
            },
        ],
    ] {
        assert!(files.edit_many("a.txt", &hash(b"abcdef"), &edits).is_err());
        assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"abcdef");
    }
}

#[test]
fn restoring_a_backup_requires_its_manifest_and_the_unchanged_post_edit_version() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    fs::write(root.path().join("a.txt"), "original\r\n").unwrap();
    let edited = files
        .edit("a.txt", &hash(b"original\r\n"), "original", "editado")
        .unwrap();
    let backup = edited["backup_path"].as_str().unwrap();
    let old_sha = edited["backup_sha256"].as_str().unwrap();
    let current_sha = edited["sha256"].as_str().unwrap();
    fs::write(root.path().join("a.txt"), "cambio externo").unwrap();
    assert!(
        files
            .restore_backup("a.txt", current_sha, backup, old_sha)
            .is_err()
    );
    assert_eq!(
        fs::read(root.path().join("a.txt")).unwrap(),
        b"cambio externo"
    );
    fs::write(root.path().join("a.txt"), "editado\r\n").unwrap();
    let restored = files
        .restore_backup("a.txt", current_sha, backup, old_sha)
        .unwrap();
    assert_eq!(restored["status"], "verified");
    assert_eq!(
        fs::read(root.path().join("a.txt")).unwrap(),
        b"original\r\n"
    );
    assert_eq!(
        fs::read(restored["backup_path"].as_str().unwrap()).unwrap(),
        b"editado\r\n"
    );
    assert!(
        files
            .restore_backup("a.txt", current_sha, backup, old_sha)
            .is_err()
    );
}

#[test]
fn restoring_an_empty_post_edit_file_and_rejecting_a_modified_backup() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    fs::write(root.path().join("a.txt"), "original").unwrap();
    let edit = files
        .edit("a.txt", &hash(b"original"), "original", "")
        .unwrap();
    let backup = edit["backup_path"].as_str().unwrap();
    fs::write(backup, "respaldo cambiado").unwrap();
    assert!(
        files
            .restore_backup("a.txt", &hash(b""), backup, &hash(b"original"))
            .is_err()
    );
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"");
    fs::write(backup, "original").unwrap();
    files
        .restore_backup("a.txt", &hash(b""), backup, &hash(b"original"))
        .unwrap();
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"original");
    assert!(
        files
            .restore_backup(
                "a.txt",
                &hash(b"original"),
                root.path().join("a.txt").to_str().unwrap(),
                &hash(b"original")
            )
            .is_err()
    );
}

#[test]
fn human_review_is_durable_but_does_not_turn_unknown_into_verified() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    fs::write(root.path().join("effect.txt"), "efecto observado").unwrap();
    journal(&files, "unknown.jsonl", &[unknown_request("effect.txt")]);
    let report = recovery::reconcile(&files, 16, 65_536, 16_384);
    assert!(report.needs_review);
    let reviewed =
        recovery::acknowledge(&files, &report.entries[0].fingerprint, 16, 65_536, 16_384).unwrap();
    assert!(!reviewed.needs_review);
    assert_eq!(reviewed.entries[0].state, State::Unknown);
    assert!(reviewed.entries[0].reviewed);
    let after_restart = recovery::reconcile(&files, 16, 65_536, 16_384);
    assert!(!after_restart.needs_review);
    assert_eq!(after_restart.entries[0].state, State::Unknown);
    fs::write(root.path().join("effect.txt"), "versión diferente").unwrap();
    let changed = recovery::reconcile(&files, 16, 65_536, 16_384);
    assert!(changed.needs_review);
    assert_ne!(
        changed.entries[0].fingerprint,
        reviewed.entries[0].fingerprint
    );
}

#[test]
fn a_changed_journal_invalidates_review_and_unknown_sessions_are_not_archivable() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    journal(&files, "unknown.jsonl", &[unknown_request("effect.txt")]);
    let report = recovery::reconcile(&files, 16, 65_536, 16_384);
    let offers = recovery::archive_offers(&files, 16, 65_536, 16_384).unwrap();
    assert!(
        offers["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["journal"] != "unknown.jsonl")
    );
    fs::OpenOptions::new()
        .append(true)
        .open(root.path().join(".pillcli/sessions/unknown.jsonl"))
        .unwrap()
        .write_all(b"{\"session\":\"real\",\"stage\":\"agent_started\"}\n")
        .unwrap();
    assert!(
        recovery::acknowledge(&files, &report.entries[0].fingerprint, 16, 65_536, 16_384).is_err()
    );
    assert!(!root.path().join(".pillcli/recovery.json").exists());
}

#[test]
fn archiving_requires_current_human_offer_and_preserves_the_original_bytes() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    journal(&files, "unknown.jsonl", &[unknown_request("effect.txt")]);
    let report = recovery::reconcile(&files, 16, 65_536, 16_384);
    recovery::acknowledge(&files, &report.entries[0].fingerprint, 16, 65_536, 16_384).unwrap();
    let before = fs::read(root.path().join(".pillcli/sessions/unknown.jsonl")).unwrap();
    let offers = recovery::archive_offers(&files, 16, 65_536, 16_384).unwrap();
    let item = offers["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["journal"] == "unknown.jsonl")
        .unwrap();
    recovery::confirm_archive(
        &files,
        item["fingerprint"].as_str().unwrap(),
        16,
        65_536,
        16_384,
    )
    .unwrap();
    assert!(!root.path().join(".pillcli/sessions/unknown.jsonl").exists());
    assert_eq!(
        fs::read(root.path().join(".pillcli/archive/sessions/unknown.jsonl")).unwrap(),
        before
    );
    assert!(!recovery::reconcile(&files, 16, 65_536, 16_384).needs_review);
}

#[test]
fn private_retention_keeps_latest_semantically_valid_backup() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    let first = transcript("válida");
    files.write_private("model.json", &first).unwrap();
    files
        .write_private("model.json", br#"{"messages":"invalid"}"#)
        .unwrap();
    files
        .write_private("model.json", &transcript("actual"))
        .unwrap();
    let offers = recovery::archive_offers(&files, 16, 65_536, 16_384).unwrap();
    let items: Vec<_> = offers["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "private_version")
        .cloned()
        .collect();
    assert!(!items.is_empty());
    for item in items {
        recovery::confirm_archive(
            &files,
            item["fingerprint"].as_str().unwrap(),
            16,
            65_536,
            16_384,
        )
        .unwrap();
    }
    fs::write(root.path().join(".pillcli/model.json"), b"truncated").unwrap();
    let recovered = files
        .read_private_checked_with("model.json", |bytes| {
            serde_json::from_slice::<Value>(bytes).is_ok_and(|value| value["messages"].is_array())
        })
        .unwrap();
    assert!(recovered.recovered);
    assert_eq!(recovered.bytes.unwrap(), first);
}

#[test]
fn archiving_another_session_does_not_revoke_an_unchanged_human_review() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    fs::write(root.path().join("safe.txt"), "completo").unwrap();
    journal(
        &files,
        "a-safe.jsonl",
        &[
            json!({"session":"safe","event":"s1","call_id":"s","stage":"requested","tool":"create_file","path":"safe.txt","target_sha256":hash(b"completo")}),
            json!({"session":"safe","event":"s2","call_id":"s","stage":"observed","tool":"create_file","path":"safe.txt","sha256":hash(b"completo"),"status":"verified"}),
        ],
    );
    journal(&files, "z-unknown.jsonl", &[unknown_request("effect.txt")]);
    let report = recovery::reconcile(&files, 16, 65_536, 16_384);
    let fingerprint = report
        .entries
        .iter()
        .find(|entry| entry.journal == "z-unknown.jsonl")
        .unwrap()
        .fingerprint
        .clone();
    recovery::acknowledge(&files, &fingerprint, 16, 65_536, 16_384).unwrap();
    let offers = recovery::archive_offers(&files, 16, 65_536, 16_384).unwrap();
    let safe = offers["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["journal"] == "a-safe.jsonl")
        .unwrap();
    recovery::confirm_archive(
        &files,
        safe["fingerprint"].as_str().unwrap(),
        16,
        65_536,
        16_384,
    )
    .unwrap();
    let after = recovery::reconcile(&files, 16, 65_536, 16_384);
    assert!(!after.needs_review);
    let unknown = after
        .entries
        .iter()
        .find(|entry| entry.journal == "z-unknown.jsonl")
        .unwrap();
    assert_eq!(unknown.state, State::Unknown);
    assert!(unknown.reviewed);
    assert_eq!(unknown.fingerprint, fingerprint);
}

#[test]
fn private_archive_rejects_changed_base_and_unexpected_members_without_moving_bytes() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    for text in ["primero", "segundo", "tercero"] {
        files
            .write_private("model.json", &transcript(text))
            .unwrap();
    }
    let offers = files.private_archive_offers().unwrap();
    let item = offers
        .iter()
        .find(|item| item["base_sha256"].is_string())
        .unwrap();
    let name = item["name"].as_str().unwrap();
    let directory = root.path().join(".pillcli").join(name);
    let base = directory.join("base");
    let original = fs::read(&base).unwrap();
    fs::write(&base, b"changed").unwrap();
    assert!(
        files
            .archive_private_version(item["fingerprint"].as_str().unwrap())
            .is_err()
    );
    assert_eq!(fs::read(&base).unwrap(), b"changed");
    assert!(
        !root
            .path()
            .join(".pillcli/archive/private")
            .join(name)
            .exists()
    );
    fs::write(&base, &original).unwrap();
    fs::write(directory.join("unexpected.txt"), "nuevo del usuario").unwrap();
    assert!(
        files
            .archive_private_version(item["fingerprint"].as_str().unwrap())
            .is_err()
    );
    assert_eq!(fs::read(&base).unwrap(), original);
    assert_eq!(
        fs::read_to_string(directory.join("unexpected.txt")).unwrap(),
        "nuevo del usuario"
    );
}

#[test]
fn archiving_old_private_generations_releases_quota_without_losing_the_latest_backup() {
    let root = tempfile::tempdir().unwrap();
    let mut config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    config.files.max_transactions = 3;
    let files = Files::new(root.path(), config.files).unwrap();
    for text in ["primero", "segundo", "tercero"] {
        files
            .write_private("model.json", &transcript(text))
            .unwrap();
    }
    assert!(
        files
            .write_private("model.json", &transcript("cuarto"))
            .is_err()
    );
    let offers = recovery::archive_offers(&files, 16, 65_536, 16_384).unwrap();
    let old = offers["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "private_version")
        .unwrap();
    recovery::confirm_archive(
        &files,
        old["fingerprint"].as_str().unwrap(),
        16,
        65_536,
        16_384,
    )
    .unwrap();
    files
        .write_private("model.json", &transcript("cuarto"))
        .unwrap();
    assert_eq!(
        files.read_private("model.json").unwrap().unwrap(),
        transcript("cuarto")
    );
    fs::write(root.path().join(".pillcli/model.json"), b"broken").unwrap();
    assert_eq!(
        files
            .read_private_checked("model.json")
            .unwrap()
            .bytes
            .unwrap(),
        transcript("tercero")
    );
}

#[test]
fn immutable_private_metadata_survives_a_truncated_state_manifest() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    let original = transcript("última base válida");
    files.write_private("model.json", &original).unwrap();
    files
        .write_private("model.json", &transcript("nueva"))
        .unwrap();
    let directory = fs::read_dir(root.path().join(".pillcli"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.join("base").is_file())
        .unwrap();
    let immutable = fs::read(directory.join("base-info.json")).unwrap();
    // Fallo de disco inyectado sobre el estado mutable, sin alterar la copia
    // inmutable sincronizada antes del traslado de la base.
    fs::write(directory.join("transaction.json"), b"{state cut").unwrap();
    fs::write(root.path().join(".pillcli/model.json"), b"{primary cut").unwrap();
    let recovered = files.read_private_checked("model.json").unwrap();
    assert!(recovered.recovered);
    assert_eq!(recovered.bytes.unwrap(), original);
    assert_eq!(
        fs::read(directory.join("base-info.json")).unwrap(),
        immutable
    );
    fs::remove_file(root.path().join(".pillcli/model.json")).unwrap();
    let recovered = files.read_private_checked("model.json").unwrap();
    assert!(recovered.recovered);
    assert_eq!(recovered.bytes.unwrap(), original);
    assert!(!root.path().join(".pillcli/model.json").exists());
    assert!(
        !files
            .private_archive_offers()
            .unwrap()
            .iter()
            .any(|item| item["name"] == directory.file_name().unwrap().to_string_lossy().as_ref())
    );
    // Un sobre JSON completo con digest incorrecto tampoco se acepta.
    let mut envelope: Value = serde_json::from_slice(&immutable).unwrap();
    envelope["metadata"]["base_sha256"] = json!(hash(b"otra base"));
    fs::write(
        directory.join("base-info.json"),
        serde_json::to_vec(&envelope).unwrap(),
    )
    .unwrap();
    assert!(
        files
            .read_private_checked("model.json")
            .unwrap()
            .bytes
            .is_none()
    );
}

#[test]
fn archived_project_backup_remains_restorable_and_rejects_stale_archive_preview() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    let before = b"a = 1\r\n";
    fs::write(root.path().join("a.txt"), before).unwrap();
    let edited = files
        .edit_many(
            "a.txt",
            &hash(before),
            &[ExactEdit {
                old: "a = 1".into(),
                new: "a = 2".into(),
            }],
        )
        .unwrap();
    let offers = recovery::archive_offers(&files, 16, 65_536, 16_384).unwrap();
    let item = offers["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == "project_transaction")
        .unwrap();
    let fingerprint = item["fingerprint"].as_str().unwrap();
    let archived = recovery::confirm_archive(&files, fingerprint, 16, 65_536, 16_384).unwrap();
    assert_eq!(archived["status"], "verified");
    assert!(!Path::new(edited["backup_path"].as_str().unwrap()).exists());
    assert!(recovery::confirm_archive(&files, fingerprint, 16, 65_536, 16_384).is_err());
    files
        .restore_backup(
            "a.txt",
            edited["sha256"].as_str().unwrap(),
            edited["backup_path"].as_str().unwrap(),
            edited["backup_sha256"].as_str().unwrap(),
        )
        .unwrap();
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), before);
}

#[test]
fn interrupted_human_restore_is_reconciled_and_not_silently_ignored() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    fs::write(root.path().join("a.txt"), b"restored").unwrap();
    journal(
        &files,
        "undo.jsonl",
        &[
            json!({"session":"undo","call_id":"u1","stage":"requested","tool":"restore_backup","path":"a.txt","expected_sha256":hash(b"edited"),"target_sha256":hash(b"restored")}),
        ],
    );
    let report = recovery::reconcile(&files, 16, 65_536, 16_384);
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].state, State::Applied, "{report:?}");
    assert!(!report.needs_review);
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"restored");
}

#[test]
fn large_project_scan_reports_limit_but_keeps_safe_private_archive_offers() {
    let root = tempfile::tempdir().unwrap();
    let mut config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
    config.files.max_entries = 16;
    let files = Files::new(root.path(), config.files).unwrap();
    fs::create_dir(root.path().join("many")).unwrap();
    for n in 0..20 {
        fs::write(root.path().join("many").join(format!("{n}.txt")), b"data").unwrap();
    }
    files
        .write_private("model.json", &transcript("first"))
        .unwrap();
    files
        .write_private("model.json", &transcript("second"))
        .unwrap();
    let offers = recovery::archive_offers(&files, 16, 65_536, 16_384).unwrap();
    assert_eq!(offers["project_scan"]["truncated"], true);
    assert!(offers["project_scan"]["error"].is_string());
    assert!(
        offers["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "private_version")
    );
    assert!(
        !offers["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "project_transaction")
    );
    for n in 0..20 {
        fs::write(root.path().join(format!("root-{n}.txt")), b"data").unwrap();
    }
    let offers = recovery::archive_offers(&files, 16, 65_536, 16_384).unwrap();
    assert_eq!(offers["retention"]["unknown"], true);
    assert!(
        offers["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "private_version")
    );
}

#[test]
fn overlapping_occurrences_are_ambiguous_even_inside_one_old_value() {
    for (before, old) in [
        ("aaa", "aa"),
        ("ababa", "aba"),
        ("\u{00e9}\u{00e9}\u{00e9}", "\u{00e9}\u{00e9}"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let files = files(root.path());
        fs::write(root.path().join("a.txt"), before).unwrap();
        assert!(
            files
                .edit("a.txt", &hash(before.as_bytes()), old, "replacement")
                .is_err()
        );
        assert_eq!(
            fs::read(root.path().join("a.txt")).unwrap(),
            before.as_bytes()
        );
        assert_eq!(files.retention_status().unwrap()["root_backups"], 0);
    }
}
