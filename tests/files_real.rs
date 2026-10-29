#![cfg(windows)]

use std::{
    fs,
    path::Path,
    sync::{Arc, Barrier},
};

use pill::files::{FileError, Files, FilesConfig};

fn config() -> FilesConfig {
    FilesConfig {
        max_file_bytes: 16_384,
        max_entries: 100,
        max_search_results: 20,
        max_transactions: 10,
        excluded_components: vec!["ignored".into()],
        extra_roots: Vec::new(),
    }
}

fn files(root: &Path) -> Files {
    Files::new(root, config()).unwrap()
}

#[test]
fn unicode_bom_crlf_create_edit_reread_and_persistent_backup() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("espacio á")).unwrap();
    let files = files(root.path());
    let path = "espacio á/niño.rs";
    let initial = "\u{feff}primero\r\nsegundo ñ\r\n";
    let created = files.create(path, initial).unwrap();
    assert_eq!(
        fs::read(root.path().join(path)).unwrap(),
        initial.as_bytes()
    );
    assert_eq!(created["status"], "verified");
    let read = files.read(path).unwrap();
    assert_eq!(read["utf8_bom"], true);
    assert_eq!(read["line_endings"], "crlf");
    let edited = files
        .edit(
            path,
            read["sha256"].as_str().unwrap(),
            "segundo ñ",
            "tercero 日本",
        )
        .unwrap();
    let actual = fs::read(root.path().join(path)).unwrap();
    assert_eq!(actual, "\u{feff}primero\r\ntercero 日本\r\n".as_bytes());
    let backup = edited["backup_path"].as_str().unwrap();
    assert_eq!(fs::read(backup).unwrap(), initial.as_bytes());
    assert_eq!(edited["backup_sha256"], read["sha256"]);
    let transaction_manifest = Path::new(backup).parent().unwrap().join("transaction.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(transaction_manifest).unwrap()).unwrap();
    assert_eq!(manifest["state"], "verified");
    assert_eq!(manifest["base_sha256"], read["sha256"]);
    let reread = files.read(path).unwrap();
    assert_eq!(reread["sha256"], edited["sha256"]);
    let listing = files.list("espacio á").unwrap();
    assert_eq!(listing["entries"].as_array().unwrap().len(), 1);
    assert_eq!(listing["omitted"], 2);
    let backup_relative = Path::new(backup)
        .strip_prefix(root.path().canonicalize().unwrap())
        .unwrap()
        .to_string_lossy();
    assert!(matches!(
        files.read(&backup_relative),
        Err(FileError::Protected(_))
    ));
}

#[test]
fn stale_base_and_ambiguous_patch_never_change_disk() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("base.txt"), "one one").unwrap();
    let files = files(root.path());
    let base = files.read("base.txt").unwrap();
    assert!(matches!(
        files.edit("base.txt", base["sha256"].as_str().unwrap(), "one", "two"),
        Err(FileError::Conflict(_))
    ));
    fs::write(root.path().join("base.txt"), "user update").unwrap();
    assert!(matches!(
        files.edit(
            "base.txt",
            base["sha256"].as_str().unwrap(),
            "one one",
            "two"
        ),
        Err(FileError::Conflict(_))
    ));
    assert_eq!(
        fs::read_to_string(root.path().join("base.txt")).unwrap(),
        "user update"
    );
    assert!(files.create("base.txt", "clobber").is_err());
    assert_eq!(
        fs::read_to_string(root.path().join("base.txt")).unwrap(),
        "user update"
    );
}

#[test]
fn two_independent_executors_cannot_both_apply_same_base() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("race.txt"), "base").unwrap();
    let first = files(root.path());
    let base = first.read("race.txt").unwrap()["sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    let second = files(root.path());
    let barrier = Arc::new(Barrier::new(2));
    let a_barrier = barrier.clone();
    let a_base = base.clone();
    let a = std::thread::spawn(move || {
        a_barrier.wait();
        first.edit("race.txt", &a_base, "base", "first")
    });
    let b = std::thread::spawn(move || {
        barrier.wait();
        second.edit("race.txt", &base, "base", "second")
    });
    let result_a = a.join().unwrap();
    let result_b = b.join().unwrap();
    assert_eq!(
        usize::from(result_a.is_ok()) + usize::from(result_b.is_ok()),
        1
    );
    let actual = fs::read_to_string(root.path().join("race.txt")).unwrap();
    assert!(actual == "first" || actual == "second");
}

#[test]
fn existing_external_writer_blocks_edit_without_lost_update() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("locked.txt");
    fs::write(&target, "base").unwrap();
    let files = files(root.path());
    let base = files.read("locked.txt").unwrap();
    let mut writer = fs::OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&target)
        .unwrap();
    assert!(
        files
            .edit(
                "locked.txt",
                base["sha256"].as_str().unwrap(),
                "base",
                "next"
            )
            .is_err()
    );
    use std::io::Write;
    writer.write_all(b"user").unwrap();
    drop(writer);
    assert_eq!(fs::read_to_string(&target).unwrap(), "user");
}

#[test]
fn traversal_ads_devices_controls_and_exclusions_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    for path in [
        "../escape",
        "C:\\outside",
        "C:relative",
        "\\\\server\\share\\x",
        "file:stream",
        "nul.txt",
        "CoM1.rs",
        "trailing.",
        "trailing ",
        ".GiT/config",
        ".pillcli/config.json",
        ".agents/a",
        ".codex/a",
        ".pill-transaction-owned/base",
        "ignored/secret",
        "line\nbreak",
    ] {
        assert!(files.create(path, "x").is_err(), "accepted {path}");
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn hardlinks_and_junctions_are_never_followed() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.txt"), "secret").unwrap();
    fs::hard_link(
        outside.path().join("secret.txt"),
        root.path().join("hard.txt"),
    )
    .unwrap();
    let files = files(root.path());
    assert!(matches!(
        files.read("hard.txt"),
        Err(FileError::Protected(_))
    ));
    // mklink /J necesita cmd porque crea un junction real sin privilegios symlink.
    // Solo creación en la muestra propia; no hay operaciones de borrado entre shells.
    let junction = root.path().join("junction");
    let status = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(files.read("junction/secret.txt").is_err());
    assert!(files.create("junction/new.txt", "escape").is_err());
    let found = files.search(".", "secret").unwrap();
    assert_eq!(found["matches"].as_array().unwrap().len(), 0);
    assert_eq!(found["omitted"], 2);
    assert!(!outside.path().join("new.txt").exists());
    // Quitar únicamente el junction creado por esta prueba, sin recursión.
    fs::remove_dir(junction).unwrap();
}

#[test]
fn explicit_protection_covers_file_and_directory() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("runtime")).unwrap();
    fs::write(root.path().join("runtime/code.rs"), "control").unwrap();
    fs::write(root.path().join("settings.json"), "{}").unwrap();
    let files = files(root.path())
        .with_protected_paths(vec![
            root.path().join("runtime"),
            root.path().join("settings.json"),
        ])
        .unwrap();
    assert!(files.read("runtime/code.rs").is_err());
    assert!(files.read("settings.json").is_err());
    assert!(files.create("runtime/new.rs", "x").is_err());
    let listing = files.list(".").unwrap();
    assert_eq!(listing["omitted"], 2);
}

#[test]
fn byte_search_entry_and_backup_limits_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), "hit\nhit\nhit\n").unwrap();
    fs::write(root.path().join("binary"), [0, 255]).unwrap();
    let mut limits = config();
    limits.max_search_results = 1;
    limits.max_transactions = 1;
    let files = Files::new(root.path(), limits).unwrap();
    assert!(files.read("binary").is_err());
    let search = files.search(".", "hit").unwrap();
    assert_eq!(search["matches"].as_array().unwrap().len(), 1);
    assert_eq!(search["truncated"], true);
    assert_eq!(search["exhaustive"], false);
    let read = files.read("a.txt").unwrap();
    let updated = files
        .edit(
            "a.txt",
            read["sha256"].as_str().unwrap(),
            "hit\nhit\nhit",
            "done",
        )
        .unwrap();
    let blocked = files.edit(
        "a.txt",
        updated["sha256"].as_str().unwrap(),
        "done",
        "other",
    );
    assert!(matches!(blocked, Err(FileError::Limit(_))), "{blocked:?}");
    assert_eq!(
        fs::read_to_string(root.path().join("a.txt")).unwrap(),
        "done\n"
    );
    let mut limits = config();
    limits.max_file_bytes = 2;
    limits.max_entries = 1;
    let limited = Files::new(root.path(), limits).unwrap();
    assert!(matches!(limited.read("a.txt"), Err(FileError::Limit(_))));
    assert!(matches!(
        limited.create("large.txt", "123"),
        Err(FileError::Limit(_))
    ));
    assert_eq!(limited.list(".").unwrap()["truncated"], true);
}

#[test]
fn private_session_journal_is_created_by_handle_and_cannot_be_accessed_by_tools() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    assert_eq!(files.session_count().unwrap(), 0);
    let (path, pins, mut journal) = files.open_session_journal("session-1.jsonl").unwrap();
    use std::io::Write;
    journal.write_all(b"{\"event\":\"verified\"}\n").unwrap();
    journal.sync_all().unwrap();
    assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
    assert!(files.read(".pillcli/sessions/session-1.jsonl").is_err());
    assert!(files.open_session_journal("session-1.jsonl").is_err());
    assert_eq!(files.session_count().unwrap(), 1);
    drop(journal);
    drop(pins);
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "{\"event\":\"verified\"}\n"
    );
}

#[test]
fn editing_preserves_restricted_owner_group_and_dacl() {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAce,
        Authorization::{GetSecurityInfo, SE_FILE_OBJECT, SetSecurityInfo},
        DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, GetKernelObjectSecurity,
        GetLengthSid, InitializeAcl, OWNER_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION,
    };
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE, LocalFree},
        Storage::FileSystem::{
            FILE_ALL_ACCESS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, WRITE_DAC,
        },
    };
    fn descriptor(file: &fs::File) -> Vec<u8> {
        let info =
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut needed = 0;
        // SAFETY: sondeo y buffer del tamaño devuelto por Windows.
        unsafe {
            GetKernelObjectSecurity(
                file.as_raw_handle(),
                info,
                std::ptr::null_mut(),
                0,
                &mut needed,
            );
        }
        assert!(needed > 0);
        let mut bytes = vec![0u8; needed as usize];
        assert_ne!(
            unsafe {
                GetKernelObjectSecurity(
                    file.as_raw_handle(),
                    info,
                    bytes.as_mut_ptr().cast(),
                    needed,
                    &mut needed,
                )
            },
            0
        );
        bytes
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("restricted.txt");
    let mut permissions = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .access_mode(GENERIC_READ | GENERIC_WRITE | WRITE_DAC)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&path)
        .unwrap();
    use std::io::Write;
    permissions.write_all(b"secret base").unwrap();
    let mut owner = std::ptr::null_mut();
    let mut security = std::ptr::null_mut();
    // SAFETY: obtener SID real del propietario evita depender del usuario del
    // entorno, que puede diferir del token de la sesión de pruebas aislada.
    assert_eq!(
        unsafe {
            GetSecurityInfo(
                permissions.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut security,
            )
        },
        0
    );
    let acl_bytes = std::mem::size_of::<ACL>() + std::mem::size_of::<ACCESS_ALLOWED_ACE>()
        - std::mem::size_of::<u32>()
        + unsafe { GetLengthSid(owner) } as usize;
    let mut acl = vec![0usize; acl_bytes.div_ceil(std::mem::size_of::<usize>())];
    assert_ne!(
        unsafe { InitializeAcl(acl.as_mut_ptr().cast(), acl_bytes as u32, ACL_REVISION) },
        0
    );
    assert_ne!(
        unsafe {
            AddAccessAllowedAce(
                acl.as_mut_ptr().cast(),
                ACL_REVISION,
                FILE_ALL_ACCESS,
                owner,
            )
        },
        0
    );
    assert_eq!(
        unsafe {
            SetSecurityInfo(
                permissions.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                acl.as_ptr().cast(),
                std::ptr::null(),
            )
        },
        0
    );
    unsafe {
        LocalFree(security);
    }
    drop(permissions);
    let before = descriptor(&fs::File::open(&path).unwrap());
    let files = files(root.path());
    let base = files.read("restricted.txt").unwrap();
    files
        .edit(
            "restricted.txt",
            base["sha256"].as_str().unwrap(),
            "secret base",
            "secret changed",
        )
        .unwrap();
    let after = descriptor(&fs::File::open(&path).unwrap());
    assert_eq!(
        before, after,
        "los permisos owner/group/DACL deben conservarse exactamente"
    );
}

#[test]
fn private_transcript_overwrites_and_journal_tail_does_not_create_a_session() {
    use std::io::Write;
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    assert_eq!(
        files.journal_tail().unwrap(),
        "No hay diario en este proyecto."
    );
    assert!(files.read_private("model.json").unwrap().is_none());
    files
        .write_private("model.json", br#"{"model":"a"}"#)
        .unwrap();
    assert_eq!(
        files.read_private("model.json").unwrap().unwrap(),
        br#"{"model":"a"}"#
    );
    files
        .write_private("model.json", br#"{"model":"b"}"#)
        .unwrap();
    assert_eq!(
        files.read_private("model.json").unwrap().unwrap(),
        br#"{"model":"b"}"#
    );
    assert!(files.write_private("otro.json", b"x").is_err());
    assert!(files.read(".pillcli/model.json").is_err());
    let (_path, pins, mut journal) = files.open_session_journal("1.jsonl").unwrap();
    writeln!(
        journal,
        r#"{{"agent":"explorer","stage":"observed","tool":"list_files","status":"verified","path":"src"}}"#
    )
    .unwrap();
    writeln!(
        journal,
        r#"{{"agent":"implementador","stage":"observed","tool":"edit_file","status":"partial","path":"src/a.py"}}"#
    )
    .unwrap();
    journal.sync_all().unwrap();
    drop(journal);
    drop(pins);
    let tail = files.journal_tail().unwrap();
    assert!(tail.contains("edit_file"), "{tail}");
    assert!(tail.contains("partial"), "{tail}");
    assert!(tail.contains("src/a.py"), "{tail}");
    assert!(tail.contains("list_files"), "{tail}");
    assert!(!tail.contains('{'), "{tail}");
    assert_eq!(files.session_count().unwrap(), 1);
}

fn operation_authorization(
    files: &Files,
    operation: &str,
    path: &str,
    destination: Option<&str>,
) -> (String, String) {
    let inspected = files.inspect_operation(path).unwrap();
    let sha = inspected["sha256"].as_str().unwrap().to_owned();
    let offer = files
        .operation_offer(operation, path, destination, &sha)
        .unwrap();
    (sha, offer["fingerprint"].as_str().unwrap().to_owned())
}

#[test]
fn move_requires_concrete_approval_preserves_binary_bytes_and_never_replaces() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("destino ñ")).unwrap();
    let bytes = [0, 255, 1, 2, 3];
    fs::write(root.path().join("base.bin"), bytes).unwrap();
    let files = files(root.path());
    let destination = "destino ñ/movido.bin";
    let (sha, approval) = operation_authorization(&files, "move", "base.bin", Some(destination));
    assert!(files.move_file("base.bin", destination, &sha, "").is_err());
    assert!(
        files
            .move_file("base.bin", "otro.bin", &sha, &approval)
            .is_err()
    );
    fs::write(root.path().join(destination), "usuario").unwrap();
    assert!(
        files
            .move_file("base.bin", destination, &sha, &approval)
            .is_err()
    );
    assert_eq!(fs::read(root.path().join(destination)).unwrap(), b"usuario");
    assert_eq!(fs::read(root.path().join("base.bin")).unwrap(), bytes);
    // Solo se retira el ocupante creado por esta prueba, en su carpeta temporal.
    fs::remove_file(root.path().join(destination)).unwrap();
    let moved = files
        .move_file("base.bin", destination, &sha, &approval)
        .unwrap();
    assert_eq!(moved["status"], "verified");
    assert_eq!(moved["source_absent"], true);
    assert!(!root.path().join("base.bin").exists());
    assert_eq!(fs::read(root.path().join(destination)).unwrap(), bytes);
}

#[test]
fn delete_file_or_empty_directory_requires_current_object_and_exact_operation() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("remove.txt"), "primero").unwrap();
    fs::create_dir(root.path().join("vacía")).unwrap();
    let files = files(root.path());
    let (old_sha, old_approval) = operation_authorization(&files, "delete", "remove.txt", None);
    assert!(files.delete("remove.txt", &old_sha, "").is_err());
    fs::write(root.path().join("remove.txt"), "posterior").unwrap();
    assert!(files.delete("remove.txt", &old_sha, &old_approval).is_err());
    assert_eq!(
        fs::read(root.path().join("remove.txt")).unwrap(),
        b"posterior"
    );
    let (sha, approval) = operation_authorization(&files, "delete", "remove.txt", None);
    assert!(
        files
            .move_file("remove.txt", "other.txt", &sha, &approval)
            .is_err()
    );
    assert_eq!(
        files.delete("remove.txt", &sha, &approval).unwrap()["absent"],
        true
    );
    assert!(!root.path().join("remove.txt").exists());
    let (sha, approval) = operation_authorization(&files, "delete", "vacía", None);
    fs::write(root.path().join("vacía/ocupante"), "nuevo").unwrap();
    assert!(files.delete("vacía", &sha, &approval).is_err());
    assert_eq!(
        fs::read(root.path().join("vacía/ocupante")).unwrap(),
        b"nuevo"
    );
    fs::remove_file(root.path().join("vacía/ocupante")).unwrap();
    assert_eq!(
        files.delete("vacía", &sha, &approval).unwrap()["kind"],
        "empty_directory"
    );
    assert!(!root.path().join("vacía").exists());
}

#[test]
fn approvals_do_not_survive_source_replacement_with_same_bytes() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source"), "igual").unwrap();
    let files = files(root.path());
    let (sha, approval) = operation_authorization(&files, "delete", "source", None);
    // Conservar el objeto previo evita que Windows reutilice su identificador.
    fs::rename(root.path().join("source"), root.path().join("previous")).unwrap();
    fs::write(root.path().join("source"), "igual").unwrap();
    assert!(files.delete("source", &sha, &approval).is_err());
    assert_eq!(fs::read(root.path().join("source")).unwrap(), b"igual");
    assert_eq!(fs::read(root.path().join("previous")).unwrap(), b"igual");
}

#[test]
fn destructive_operations_reject_links_protected_paths_and_external_writers() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "intacto").unwrap();
    fs::hard_link(outside.path().join("secret"), root.path().join("link")).unwrap();
    fs::write(root.path().join("locked"), "base").unwrap();
    fs::write(root.path().join("protected"), "control").unwrap();
    let files = files(root.path())
        .with_protected_paths(vec![root.path().join("protected")])
        .unwrap();
    assert!(files.inspect_operation("link").is_err());
    assert!(files.inspect_operation("protected").is_err());
    assert!(files.inspect_operation(".").is_err());
    assert!(files.inspect_operation("../secret").is_err());
    let (sha, approval) = operation_authorization(&files, "delete", "locked", None);
    let writer = fs::OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(root.path().join("locked"))
        .unwrap();
    assert!(files.delete("locked", &sha, &approval).is_err());
    drop(writer);
    assert_eq!(fs::read(root.path().join("locked")).unwrap(), b"base");
    assert_eq!(fs::read(outside.path().join("secret")).unwrap(), b"intacto");
}

#[test]
fn private_hardlink_is_rejected_before_any_truncation() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.json"), "NO CAMBIAR").unwrap();
    fs::create_dir(root.path().join(".pillcli")).unwrap();
    fs::hard_link(
        outside.path().join("secret.json"),
        root.path().join(".pillcli/model.json"),
    )
    .unwrap();
    let files = files(root.path());
    assert!(matches!(
        files.write_private("model.json", b"{}"),
        Err(FileError::Protected(_))
    ));
    assert_eq!(
        fs::read(outside.path().join("secret.json")).unwrap(),
        b"NO CAMBIAR"
    );
}

#[test]
fn recovery_journal_reads_enforce_limits_and_refuse_links() {
    use std::io::Write;
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path());
    let (_path, pins, mut journal) = files.open_session_journal("one.jsonl").unwrap();
    journal.write_all(b"{}\n").unwrap();
    drop(journal);
    drop(pins);
    assert_eq!(
        files.session_journals(2, 3).unwrap(),
        vec![("one.jsonl".into(), b"{}\n".to_vec())]
    );
    assert!(files.session_journals(2, 2).is_err());
    let (_path, pins, journal) = files.open_session_journal("two.jsonl").unwrap();
    drop(journal);
    drop(pins);
    assert!(files.session_journals(1, 10).is_err());
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("private"), "secret").unwrap();
    fs::hard_link(
        outside.path().join("private"),
        root.path().join(".pillcli/sessions/link.jsonl"),
    )
    .unwrap();
    assert!(files.session_journals(3, 10).is_err());
    assert_eq!(fs::read(outside.path().join("private")).unwrap(), b"secret");
}
