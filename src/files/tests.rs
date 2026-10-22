use super::*;

fn config() -> FilesConfig {
    FilesConfig {
        max_file_bytes: 8192,
        max_entries: 10_000,
        max_search_results: 10,
        max_transactions: 10,
        excluded_components: Vec::new(),
        extra_roots: Vec::new(),
    }
}

#[test]
fn publication_collision_preserves_occupant_base_and_staged_data() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("target"), "base").unwrap();
    let files = Files::new(root.path(), config()).unwrap();
    let resolved = files.resolve("target", false).unwrap();
    let original = platform::open_file_at(&resolved.parent, &resolved.leaf, true).unwrap();
    let transaction = files.transaction(&resolved.parent, root.path()).unwrap();
    let mut staged = platform::create_file_at(&transaction.pin, "new".as_ref()).unwrap();
    staged.write_all(b"next").unwrap();
    platform::rename_at(&original, &transaction.pin, "base".as_ref()).unwrap();
    fs::write(root.path().join("target"), "external occupant").unwrap();
    let backup = transaction.path.join("base");
    let result = publish_staged(
        &original,
        &staged,
        &resolved.parent,
        &resolved.leaf,
        &backup,
        "target",
    );
    assert!(
        matches!(result, Err(FileError::Partial { .. })),
        "{result:?}"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("target")).unwrap(),
        "external occupant"
    );
    drop(original);
    drop(staged);
    assert_eq!(fs::read_to_string(backup).unwrap(), "base");
    assert_eq!(
        fs::read_to_string(transaction.path.join("new")).unwrap(),
        "next"
    );
}

#[test]
fn failed_publication_rolls_back_only_into_absent_destination() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("target"), "base").unwrap();
    let files = Files::new(root.path(), config()).unwrap();
    let resolved = files.resolve("target", false).unwrap();
    let original = platform::open_file_at(&resolved.parent, &resolved.leaf, true).unwrap();
    let transaction = files.transaction(&resolved.parent, root.path()).unwrap();
    let mut writable = platform::create_file_at(&transaction.pin, "new".as_ref()).unwrap();
    writable.write_all(b"next").unwrap();
    drop(writable);
    // Handle real sin DELETE: su publicación debe fallar por permisos.
    let staged = platform::open_file_at(&transaction.pin, "new".as_ref(), false).unwrap();
    platform::rename_at(&original, &transaction.pin, "base".as_ref()).unwrap();
    let backup = transaction.path.join("base");
    let result = publish_staged(
        &original,
        &staged,
        &resolved.parent,
        &resolved.leaf,
        &backup,
        "target",
    );
    assert!(matches!(result, Err(FileError::Conflict(_))), "{result:?}");
    drop(original);
    assert_eq!(
        fs::read_to_string(root.path().join("target")).unwrap(),
        "base"
    );
    assert!(!backup.exists());
}

#[test]
fn live_write_and_delete_handles_are_excluded_and_directory_cannot_be_renamed() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("target"), "base").unwrap();
    let files = Files::new(root.path(), config()).unwrap();
    let resolved = files.resolve("target", false).unwrap();
    let original = platform::open_file_at(&resolved.parent, &resolved.leaf, true).unwrap();
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(root.path().join("target"))
            .is_err()
    );
    assert!(fs::rename(root.path().join("target"), root.path().join("stolen")).is_err());
    assert!(fs::rename(root.path(), root.path().with_extension("stolen")).is_err());
    drop(original);
}

#[test]
fn empty_pinned_directory_changed_to_junction_fails_closed_on_relative_io() {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::{
        Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
            MAXIMUM_REPARSE_DATA_BUFFER_SIZE,
        },
        System::{
            IO::DeviceIoControl,
            Ioctl::{FSCTL_DELETE_REPARSE_POINT, FSCTL_GET_REPARSE_POINT, FSCTL_SET_REPARSE_POINT},
        },
    };
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let junction = root.path().join("sample-junction");
    let empty = root.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let status = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(status.status.success());
    let files = Files::new(root.path(), config()).unwrap();
    let resolved = files.resolve("empty/new.txt", false).unwrap();
    let reparse_source = fs::OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&junction)
        .unwrap();
    let mut reparse = vec![0u8; MAXIMUM_REPARSE_DATA_BUFFER_SIZE as usize];
    let mut returned = 0;
    // SAFETY: buffers vivos de tamaño indicado y handles válidos.
    assert_ne!(
        unsafe {
            DeviceIoControl(
                reparse_source.as_raw_handle(),
                FSCTL_GET_REPARSE_POINT,
                std::ptr::null(),
                0,
                reparse.as_mut_ptr().cast(),
                reparse.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        },
        0
    );
    reparse.truncate(returned as usize);
    let attacker = fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&empty)
        .unwrap();
    assert_ne!(
        unsafe {
            DeviceIoControl(
                attacker.as_raw_handle(),
                FSCTL_SET_REPARSE_POINT,
                reparse.as_ptr().cast(),
                reparse.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        },
        0,
        "mutación junction no ejercitada: {}",
        std::io::Error::last_os_error()
    );
    assert!(platform::create_file_at(&resolved.parent, &resolved.leaf).is_err());
    assert!(platform::open_file_at(&resolved.parent, "secret.txt".as_ref(), false).is_err());
    assert!(platform::entries(&resolved.parent, 10).is_err());
    let root_handle = files._root_pins.last().unwrap();
    let staged = platform::create_file_at(root_handle, "staged".as_ref()).unwrap();
    assert!(platform::rename_at(&staged, &resolved.parent, "published".as_ref()).is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    // Quitar solo los reparse creados en esta prueba usando sus handles.
    let mut deletion = [0u8; 8];
    deletion[..4].copy_from_slice(&reparse[..4]);
    assert_ne!(
        unsafe {
            DeviceIoControl(
                attacker.as_raw_handle(),
                FSCTL_DELETE_REPARSE_POINT,
                deletion.as_ptr().cast(),
                deletion.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        },
        0
    );
    drop(attacker);
    drop(reparse_source);
    fs::remove_dir(junction).unwrap();
}

#[test]
fn concurrent_enumerations_have_independent_cursors_across_buffers() {
    use std::sync::{Arc, Barrier};
    let root = tempfile::tempdir().unwrap();
    let count = 1400;
    for index in 0..count {
        fs::write(
            root.path().join(format!("entry-{index:04}-long-name.txt")),
            "",
        )
        .unwrap();
    }
    let files = Arc::new(Files::new(root.path(), config()).unwrap());
    let barrier = Arc::new(Barrier::new(2));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let files = files.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            let listing = files.list(".").unwrap();
            assert_eq!(listing["truncated"], false);
            let entries = listing["entries"].as_array().unwrap();
            assert_eq!(entries.len(), count);
            let unique: std::collections::HashSet<_> = entries
                .iter()
                .map(|entry| entry["path"].as_str().unwrap())
                .collect();
            assert_eq!(unique.len(), count);
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }
}
