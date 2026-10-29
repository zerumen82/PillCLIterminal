//! Registered commands. A repository proposes definitions; only the interactive
//! composition layer may grant their current fingerprints in the private store.
//!
//! This is an authorization boundary, not an operating-system sandbox. Approved
//! programs can access whatever their Windows user can access.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub id: String,
    /// Absolute executable path. No PATH lookup or implicit shell is performed.
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Relative to the canonical project root.
    pub cwd: PathBuf,
    /// Scripts/build definitions whose changes require renewed consent.
    pub watched_files: Vec<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandsConfig {
    pub timeout_ms: u64,
    pub poll_interval_ms: u64,
    pub max_output_bytes: usize,
    pub max_watched_bytes: u64,
    pub commands: Vec<CommandSpec>,
}

impl CommandsConfig {
    pub fn validate(&self) -> Result<(), CommandError> {
        if !(1..=3_600_000).contains(&self.timeout_ms)
            || !(1..=1_000).contains(&self.poll_interval_ms)
            || !(1..=16_777_216).contains(&self.max_output_bytes)
            || !(1..=1_073_741_824).contains(&self.max_watched_bytes)
            || self.commands.len() > 128
        {
            return Err(CommandError::Invalid(
                "límites de comandos fuera de rango (hasta 1 h, 16 MiB de salida y 1 GiB observado)".into(),
            ));
        }
        if self.poll_interval_ms > self.timeout_ms {
            return Err(CommandError::Invalid(
                "poll_interval_ms supera timeout_ms".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        for spec in &self.commands {
            if spec.id.is_empty() || spec.id.chars().any(char::is_control) || !ids.insert(&spec.id)
            {
                return Err(CommandError::Invalid(
                    "id de comando vacío, repetido o con controles".into(),
                ));
            }
            if !spec.program.is_absolute()
                || spec.args.len() > 256
                || spec.watched_files.len() > 128
                || spec
                    .args
                    .iter()
                    .any(|arg| arg.contains('\0') || arg.len() > 32767)
            {
                return Err(CommandError::Invalid(format!(
                    "{}: programa absoluto y argumentos sin NUL requeridos",
                    spec.id
                )));
            }
            relative_path(&spec.cwd)?;
            for file in &spec.watched_files {
                relative_path(file)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("Comando: {0}")]
    Invalid(String),
    #[error("Comando no registrado: {0}")]
    Unknown(String),
    #[error("El usuario debe conceder la propuesta actual de {0}")]
    NotGranted(String),
    #[error("La propuesta de {0} cambió; vuelve a mostrarla antes de concederla")]
    StaleProposal(String),
    #[error("Ejecución de comandos no disponible en esta plataforma")]
    Unsupported,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("Concesiones JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, Serialize)]
pub struct CommandProposal {
    pub spec: CommandSpec,
    pub resolved_cwd: PathBuf,
    pub observed_files: Vec<ObservedFile>,
    pub fingerprint: String,
    pub granted: bool,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ObservedFile {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct CommandResult {
    pub id: String,
    pub fingerprint: String,
    pub exit_code: Option<u32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub stdout_lossy: bool,
    pub stderr_lossy: bool,
    pub timed_out: bool,
    pub cancelled: bool,
    pub elapsed_ms: u128,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Grants {
    pub projects: BTreeMap<PathBuf, BTreeMap<String, String>>,
    #[serde(default)]
    pub mcp: BTreeMap<PathBuf, BTreeMap<String, String>>,
    #[serde(default)]
    pub exclusive: BTreeMap<PathBuf, String>,
}

#[derive(Clone)]
pub struct CommandRunner {
    root: PathBuf,
    config: CommandsConfig,
    trust_path: PathBuf,
}

struct Prepared {
    spec: CommandSpec,
    cwd: PathBuf,
    fingerprint: String,
    observed_files: Vec<ObservedFile>,
    // Open handles deny writes/deletes on Windows until execution has finished.
    _locks: Vec<File>,
}

impl CommandRunner {
    pub fn new(
        root: &Path,
        config: CommandsConfig,
        trust_path: &Path,
    ) -> Result<Self, CommandError> {
        config.validate()?;
        reject_links(root)?;
        let root = root.canonicalize()?;
        if !root.is_dir() || !trust_path.is_absolute() {
            return Err(CommandError::Invalid(
                "raíz de directorio y concesiones absolutas requeridas".into(),
            ));
        }
        let parent = trust_path
            .parent()
            .ok_or_else(|| CommandError::Invalid("ruta de concesiones sin padre".into()))?;
        reject_links(parent)?;
        if trust_path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        {
            return Err(CommandError::Invalid(
                "ruta privada debe ser absoluta y normalizada".into(),
            ));
        }
        let existing = parent
            .ancestors()
            .find(|ancestor| ancestor.exists())
            .ok_or_else(|| CommandError::Invalid("ruta privada sin ancestro existente".into()))?;
        let trust_path = existing.canonicalize()?.join(
            trust_path
                .strip_prefix(existing)
                .map_err(|_| CommandError::Invalid("ruta privada inválida".into()))?,
        );
        if contained(&trust_path, &root) {
            return Err(CommandError::Invalid(
                "el almacén privado de concesiones debe estar fuera de la raíz del modelo".into(),
            ));
        }
        reject_links(&trust_path)?;
        let runner = Self {
            root,
            config,
            trust_path,
        };
        runner.read_grants()?;
        Ok(runner)
    }

    pub fn has_definitions(&self) -> bool {
        !self.config.commands.is_empty()
    }

    pub fn load_grants(&self) -> Result<Grants, CommandError> {
        self.read_grants()
    }

    pub fn save_grants(&self, grants: &Grants) -> Result<(), CommandError> {
        self.write_grants(grants)
    }

    pub fn project_root(&self) -> &Path {
        &self.root
    }

    pub fn proposals(&self) -> Result<Vec<CommandProposal>, CommandError> {
        let grants = self.read_grants()?;
        self.config
            .commands
            .iter()
            .map(|spec| {
                let prepared = self.prepare(&spec.id)?;
                let granted = grants
                    .projects
                    .get(&self.root)
                    .and_then(|commands| commands.get(&spec.id))
                    == Some(&prepared.fingerprint);
                Ok(CommandProposal {
                    spec: spec.clone(),
                    resolved_cwd: prepared.cwd,
                    observed_files: prepared.observed_files,
                    fingerprint: prepared.fingerprint,
                    granted,
                    timeout_ms: self.config.timeout_ms,
                    max_output_bytes: self.config.max_output_bytes,
                })
            })
            .collect()
    }

    /// Revalida la definición y concesión asociadas a una prueba ya ejecutada.
    pub fn verify_fingerprint(&self, id: &str, fingerprint: &str) -> Result<(), CommandError> {
        let prepared = self.prepare(id)?;
        let grants = self.read_grants()?;
        if prepared.fingerprint != fingerprint
            || grants
                .projects
                .get(&self.root)
                .and_then(|commands| commands.get(id))
                != Some(&prepared.fingerprint)
        {
            return Err(CommandError::StaleProposal(id.into()));
        }
        Ok(())
    }

    /// Call only after the user explicitly approves this displayed fingerprint.
    /// Never register this method as a model tool.
    pub fn grant(&self, id: &str, displayed_fingerprint: &str) -> Result<(), CommandError> {
        // This is the first explicit user action allowed to create private state.
        reject_links(&self.trust_path)?;
        fs::create_dir_all(self.trust_path.parent().expect("validated parent"))?;
        let prepared = self.prepare(id)?;
        if prepared.fingerprint != displayed_fingerprint {
            return Err(CommandError::StaleProposal(id.into()));
        }
        let mut grants = self.read_grants()?;
        grants
            .projects
            .entry(self.root.clone())
            .or_default()
            .insert(id.into(), prepared.fingerprint);
        self.write_grants(&grants)
    }

    pub fn revoke(&self, id: &str) -> Result<(), CommandError> {
        let mut grants = self.read_grants()?;
        if let Some(commands) = grants.projects.get_mut(&self.root) {
            commands.remove(id);
        }
        self.write_grants(&grants)
    }

    pub async fn execute(
        &self,
        id: &str,
        cancellation: Arc<AtomicBool>,
    ) -> Result<CommandResult, CommandError> {
        let runner = self.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || runner.execute_blocking(&id, cancellation))
            .await
            .map_err(|error| CommandError::Invalid(format!("trabajador de comando: {error}")))?
    }

    fn execute_blocking(
        &self,
        id: &str,
        cancellation: Arc<AtomicBool>,
    ) -> Result<CommandResult, CommandError> {
        let prepared = self.prepare(id)?;
        let grants = self.read_grants()?;
        if grants
            .projects
            .get(&self.root)
            .and_then(|commands| commands.get(id))
            != Some(&prepared.fingerprint)
        {
            return Err(CommandError::NotGranted(id.into()));
        }
        if cancellation.load(Ordering::Acquire) {
            return Ok(CommandResult {
                id: id.into(),
                fingerprint: prepared.fingerprint,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_lossy: false,
                stderr_lossy: false,
                timed_out: false,
                cancelled: true,
                elapsed_ms: 0,
            });
        }
        #[cfg(windows)]
        {
            windows::execute(prepared, &self.config, cancellation)
        }
        #[cfg(not(windows))]
        {
            let _ = (prepared, cancellation);
            Err(CommandError::Unsupported)
        }
    }

    fn prepare(&self, id: &str) -> Result<Prepared, CommandError> {
        let spec = self
            .config
            .commands
            .iter()
            .find(|spec| spec.id == id)
            .ok_or_else(|| CommandError::Unknown(id.into()))?
            .clone();
        let mut locks = Vec::new();
        pin_directories(&self.root.join(&spec.cwd), &mut locks)?;
        reject_links(&self.root)?;
        let cwd = self.project_path(&spec.cwd)?;
        if !cwd.is_dir() {
            return Err(CommandError::Invalid("cwd no es directorio".into()));
        }
        reject_links(&spec.program)?;
        let program = spec.program.canonicalize()?;
        let mut paths = BTreeSet::from([program]);
        for file in &spec.watched_files {
            paths.insert(self.project_path(file)?);
        }
        // Direct script/file arguments are watched even when the proposal omitted
        // them. Transitive build inputs remain an explicit user-reviewed list.
        for arg in &spec.args {
            let path = Path::new(arg);
            if !path.is_absolute() && !arg.starts_with('-') && cwd.join(path).is_file() {
                let path = cwd.join(path);
                reject_links(&path)?;
                let path = path.canonicalize()?;
                if !contained(&path, &self.root) {
                    return Err(CommandError::Invalid(
                        "argumento de archivo fuera de la raíz".into(),
                    ));
                }
                paths.insert(path);
            } else if path.is_absolute() && path.is_file() {
                reject_links(path)?;
                let path = path.canonicalize()?;
                if !contained(&path, &self.root) {
                    return Err(CommandError::Invalid(
                        "argumento de archivo fuera de la raíz".into(),
                    ));
                }
                paths.insert(path);
            }
        }
        if !paths
            .iter()
            .any(|path| contained(path, &cwd) && path != &cwd)
        {
            return Err(CommandError::Invalid(format!(
                "{}: cwd necesita un archivo observado dentro de su árbol; añade el script/build/test a watched_files",
                spec.id
            )));
        }
        let mut hash = Sha256::new();
        hash.update(serde_json::to_vec(&(
            &self.root,
            &spec,
            &self.config.timeout_ms,
            &self.config.max_output_bytes,
        ))?);
        let mut remaining = self.config.max_watched_bytes;
        let mut observed_files = Vec::new();
        for path in paths {
            pin_directories(
                path.parent()
                    .ok_or_else(|| CommandError::Invalid("archivo sin padre".into()))?,
                &mut locks,
            )?;
            reject_links(&path)?;
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options
                    .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
                    .custom_flags(
                        windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
                    );
            }
            let mut file = options.open(&path)?;
            check_metadata_link(&file.metadata()?, &path)?;
            verify_opened_path(&file, &path)?;
            if !file.metadata()?.is_file() || file.metadata()?.len() > self.config.max_watched_bytes
            {
                return Err(CommandError::Invalid(format!(
                    "archivo observado inválido o demasiado grande: {}",
                    path.display()
                )));
            }
            hash.update(serde_json::to_vec(&path)?);
            hash.update(file.metadata()?.len().to_le_bytes());
            let mut buffer = [0_u8; 8192];
            let mut file_hash = Sha256::new();
            loop {
                let size = file.read(&mut buffer)?;
                if size == 0 {
                    break;
                }
                if size as u64 > remaining {
                    return Err(CommandError::Invalid(
                        "archivo observado creció sobre el límite".into(),
                    ));
                }
                remaining -= size as u64;
                hash.update(&buffer[..size]);
                file_hash.update(&buffer[..size]);
            }
            observed_files.push(ObservedFile {
                path,
                sha256: format!("{:x}", file_hash.finalize()),
            });
            locks.push(file);
        }
        // A locked descendant keeps cwd and each of its pinned ancestors
        // nonempty. Windows rejects redirection reparse points on nonempty
        // directories. Recheck after opening the descendants, closing the
        // preparation window before this invariant becomes established.
        for locked in &locks {
            check_metadata_link(&locked.metadata()?, &cwd)?;
        }
        Ok(Prepared {
            spec,
            cwd,
            fingerprint: format!("{:x}", hash.finalize()),
            observed_files,
            _locks: locks,
        })
    }

    fn project_path(&self, relative: &Path) -> Result<PathBuf, CommandError> {
        relative_path(relative)?;
        let path = self.root.join(relative);
        reject_links(&path)?;
        let path = path.canonicalize()?;
        if !contained(&path, &self.root) {
            return Err(CommandError::Invalid("ruta fuera de raíz".into()));
        }
        Ok(path)
    }

    fn read_grants(&self) -> Result<Grants, CommandError> {
        let mut locks = Vec::new();
        let parent = self.trust_path.parent().expect("validated parent");
        if !parent.exists() {
            reject_links(parent)?;
            return Ok(Grants::default());
        }
        pin_directories(parent, &mut locks)?;
        reject_links(&self.trust_path)?;
        match File::open(&self.trust_path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(self.config.max_watched_bytes.saturating_add(1))
                    .read_to_end(&mut bytes)?;
                if bytes.len() as u64 > self.config.max_watched_bytes {
                    return Err(CommandError::Invalid(
                        "almacén de concesiones excede el límite".into(),
                    ));
                }
                Ok(serde_json::from_slice(&bytes)?)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Grants::default()),
            Err(error) => Err(error.into()),
        }
    }

    fn write_grants(&self, grants: &Grants) -> Result<(), CommandError> {
        let mut locks = Vec::new();
        reject_links(&self.trust_path)?;
        if !self.trust_path.parent().expect("validated parent").is_dir() {
            fs::create_dir_all(self.trust_path.parent().expect("validated parent"))?;
        }
        pin_directories(
            self.trust_path.parent().expect("validated parent"),
            &mut locks,
        )?;
        reject_links(&self.trust_path)?;
        let bytes = serde_json::to_vec_pretty(grants)?;
        if bytes.len() as u64 > self.config.max_watched_bytes {
            return Err(CommandError::Invalid(
                "almacén de concesiones excede el límite".into(),
            ));
        }
        let mut temporary =
            tempfile::NamedTempFile::new_in(self.trust_path.parent().expect("validated parent"))
                .map_err(|error| {
                    CommandError::Invalid(format!("crear temporal privado: {error}"))
                })?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.trust_path).map_err(|error| {
            CommandError::Invalid(format!("publicar concesiones: {}", error.error))
        })?;
        Ok(())
    }
}

fn contained(path: &Path, root: &Path) -> bool {
    let mut path_components = path.components();
    root.components().all(|expected| {
        path_components.next().is_some_and(|actual| {
            #[cfg(windows)]
            {
                use std::os::windows::ffi::OsStrExt;
                use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
                let actual: Vec<u16> = actual.as_os_str().encode_wide().collect();
                let expected: Vec<u16> = expected.as_os_str().encode_wide().collect();
                if actual.len() > i32::MAX as usize || expected.len() > i32::MAX as usize {
                    return false;
                }
                unsafe {
                    CompareStringOrdinal(
                        actual.as_ptr(),
                        actual.len() as i32,
                        expected.as_ptr(),
                        expected.len() as i32,
                        1,
                    ) == CSTR_EQUAL
                }
            }
            #[cfg(not(windows))]
            {
                actual == expected
            }
        })
    })
}

fn relative_path(path: &Path) -> Result<(), CommandError> {
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        || path.to_string_lossy().contains(':')
        || path.to_string_lossy().contains('\0')
    {
        return Err(CommandError::Invalid(format!(
            "ruta relativa sin escapes requerida: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(super) fn reject_links(path: &Path) -> Result<(), CommandError> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => check_metadata_link(&metadata, ancestor)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(super) fn check_metadata_link(
    metadata: &fs::Metadata,
    path: &Path,
) -> Result<(), CommandError> {
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    };
    #[cfg(not(windows))]
    let reparse = metadata.file_type().is_symlink();
    if reparse {
        return Err(CommandError::Invalid(format!(
            "enlace/junction no admitido: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(super) fn pin_directories(path: &Path, locks: &mut Vec<File>) -> Result<(), CommandError> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
            FILE_SHARE_WRITE,
        };
        let ancestors: Vec<_> = path.ancestors().collect();
        for directory in ancestors.into_iter().rev() {
            let file = OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(directory)?;
            check_metadata_link(&file.metadata()?, directory)?;
            verify_opened_path(&file, directory)?;
            if !file.metadata()?.is_dir() {
                return Err(CommandError::Invalid("ancestro no es directorio".into()));
            }
            locks.push(file);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = locks;
        reject_links(path)?;
    }
    Ok(())
}

pub(super) fn verify_opened_path(file: &File, expected: &Path) -> Result<(), CommandError> {
    #[cfg(windows)]
    {
        use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
        let required =
            unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), std::ptr::null_mut(), 0, 0) };
        if required == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut units = vec![0_u16; required as usize + 1];
        let written = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                units.as_mut_ptr(),
                units.len() as u32,
                0,
            )
        };
        if written == 0 || written as usize >= units.len() {
            return Err(std::io::Error::last_os_error().into());
        }
        let actual = PathBuf::from(std::ffi::OsString::from_wide(&units[..written as usize]));
        let expected = expected.components().collect::<PathBuf>();
        // Ancestors of canonical \?\ paths retain their prefix; caller paths
        // are canonical before pinning. Root-only paths also normalize here.
        if !(contained(&actual, &expected) && contained(&expected, &actual)) {
            return Err(CommandError::Invalid(format!(
                "ruta cambió al abrir {} (observada {})",
                expected.display(),
                actual.display()
            )));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (file, expected);
    }
    Ok(())
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{
        ffi::OsStr,
        mem::{size_of, zeroed},
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
        },
        ptr, thread,
    };
    use windows_sys::Win32::{
        Foundation::{
            HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation, WAIT_OBJECT_0,
            WAIT_TIMEOUT,
        },
        Security::SECURITY_ATTRIBUTES,
        Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
            Pipes::CreatePipe,
            Threading::{
                CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessW, DeleteProcThreadAttributeList,
                EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
                InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW,
                TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
            },
        },
    };

    fn wide(value: &OsStr) -> Result<Vec<u16>, CommandError> {
        let mut units: Vec<_> = value.encode_wide().collect();
        if units.contains(&0) {
            return Err(CommandError::Invalid("NUL en ruta/argumento".into()));
        }
        units.push(0);
        Ok(units)
    }

    fn process_cwd(path: &Path) -> std::ffi::OsString {
        // Canonical Windows paths use the verbatim prefix. Passing that prefix
        // as a process cwd breaks path providers in programs such as PowerShell.
        // Keep canonical paths for policy/locks, use the equivalent DOS spelling
        // only for CreateProcessW's working-directory parameter.
        let units: Vec<u16> = path.as_os_str().encode_wide().collect();
        let prefix: Vec<u16> = "\\\\?\\".encode_utf16().collect();
        if let Some(tail) = units.strip_prefix(prefix.as_slice()) {
            let unc: Vec<u16> = "UNC\\".encode_utf16().collect();
            if let Some(tail) = tail.strip_prefix(unc.as_slice()) {
                use std::os::windows::ffi::OsStringExt;
                let mut dos: Vec<u16> = "\\\\".encode_utf16().collect();
                dos.extend_from_slice(tail);
                return std::ffi::OsString::from_wide(&dos);
            }
            use std::os::windows::ffi::OsStringExt;
            return std::ffi::OsString::from_wide(tail);
        }
        path.as_os_str().to_owned()
    }

    // Windows CommandLineToArgvW/CRT argument quoting. The application path is
    // supplied separately to CreateProcessW; no command interpreter is inserted.
    fn quoted(value: &OsStr) -> Vec<u16> {
        let mut result = vec![b'"' as u16];
        let mut slashes = 0;
        for unit in value.encode_wide() {
            if unit == b'\\' as u16 {
                slashes += 1;
                continue;
            }
            if unit == b'"' as u16 {
                result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
            } else {
                result.extend(std::iter::repeat_n(b'\\' as u16, slashes));
            }
            slashes = 0;
            result.push(unit);
        }
        result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        result.push(b'"' as u16);
        result
    }

    fn checked_handle(handle: HANDLE) -> Result<OwnedHandle, CommandError> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(std::io::Error::last_os_error().into())
        } else {
            Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
        }
    }

    fn pipe(attributes: &SECURITY_ATTRIBUTES) -> Result<(OwnedHandle, OwnedHandle), CommandError> {
        let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
        if unsafe { CreatePipe(&mut read, &mut write, attributes, 0) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let read = checked_handle(read)?;
        let write = checked_handle(write)?;
        if unsafe { SetHandleInformation(read.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok((read, write))
    }

    struct Attributes {
        _buffer: Vec<usize>,
        pointer: windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST,
    }
    impl Drop for Attributes {
        fn drop(&mut self) {
            unsafe {
                DeleteProcThreadAttributeList(self.pointer);
            }
        }
    }

    impl Attributes {
        fn new(handles: &mut [HANDLE]) -> Result<Self, CommandError> {
            let mut size = 0;
            unsafe {
                InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size);
            }
            let mut buffer = vec![0_usize; size.div_ceil(size_of::<usize>())];
            let pointer = buffer.as_mut_ptr().cast();
            if unsafe { InitializeProcThreadAttributeList(pointer, 1, 0, &mut size) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let attributes = Self {
                _buffer: buffer,
                pointer,
            };
            if unsafe {
                UpdateProcThreadAttribute(
                    pointer,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    handles.as_mut_ptr().cast(),
                    std::mem::size_of_val(handles),
                    ptr::null_mut(),
                    ptr::null(),
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(attributes)
        }
    }

    struct Child {
        process: OwnedHandle,
        // Closing our private job kills only its descendants, including children
        // that outlive the registered executable. Breakaway is not enabled.
        _job: OwnedHandle,
    }
    impl Drop for Child {
        fn drop(&mut self) {
            unsafe {
                TerminateProcess(self.process.as_raw_handle(), 1);
            }
        }
    }

    fn start(prepared: &Prepared) -> Result<(Child, OwnedHandle, OwnedHandle), CommandError> {
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: ptr::null_mut(),
            bInheritHandle: 1,
        };
        let (out_read, out_write) = pipe(&attributes)?;
        let (err_read, err_write) = pipe(&attributes)?;
        let nul = wide(OsStr::new("NUL"))?;
        let input = checked_handle(unsafe {
            CreateFileW(
                nul.as_ptr(),
                windows_sys::Win32::Foundation::GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &attributes,
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            )
        })?;
        let job = checked_handle(unsafe { CreateJobObjectW(ptr::null(), ptr::null()) })?;
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut handles = [
            input.as_raw_handle(),
            out_write.as_raw_handle(),
            err_write.as_raw_handle(),
        ];
        let process_attributes = Attributes::new(&mut handles)?;
        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = input.as_raw_handle();
        startup.StartupInfo.hStdOutput = out_write.as_raw_handle();
        startup.StartupInfo.hStdError = err_write.as_raw_handle();
        startup.lpAttributeList = process_attributes.pointer;
        let application = wide(prepared.spec.program.as_os_str())?;
        let cwd = wide(&process_cwd(&prepared.cwd))?;
        let mut command = quoted(prepared.spec.program.as_os_str());
        for argument in &prepared.spec.args {
            command.push(b' ' as u16);
            command.extend(quoted(OsStr::new(argument)));
        }
        if command.len() >= 32767 {
            return Err(CommandError::Invalid(
                "línea de comando supera el límite Win32".into(),
            ));
        }
        command.push(0);
        let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
        if unsafe {
            CreateProcessW(
                application.as_ptr(),
                command.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                CREATE_NO_WINDOW | CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
                ptr::null(),
                cwd.as_ptr(),
                &startup.StartupInfo,
                &mut process,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let process_handle = checked_handle(process.hProcess)?;
        let thread_handle = checked_handle(process.hThread)?;
        let child = Child {
            process: process_handle,
            _job: job,
        };
        if unsafe {
            AssignProcessToJobObject(child._job.as_raw_handle(), child.process.as_raw_handle())
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        if unsafe { ResumeThread(thread_handle.as_raw_handle()) } == u32::MAX {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok((child, out_read, err_read))
    }

    fn capture(handle: OwnedHandle, limit: usize) -> std::io::Result<(String, bool, bool)> {
        let mut file = File::from(handle);
        let mut result = Vec::new();
        let mut truncated = false;
        let mut buffer = [0_u8; 8192];
        loop {
            let size = file.read(&mut buffer)?;
            if size == 0 {
                break;
            }
            let keep = size.min(limit.saturating_sub(result.len()));
            result.extend_from_slice(&buffer[..keep]);
            truncated |= keep < size;
        }
        let decoded = String::from_utf8_lossy(&result);
        let lossy = matches!(decoded, std::borrow::Cow::Owned(_));
        let mut text = decoded.into_owned();
        while text.len() > limit {
            text.pop();
            truncated = true;
        }
        Ok((text, truncated, lossy))
    }

    pub(super) fn execute(
        prepared: Prepared,
        config: &CommandsConfig,
        cancellation: Arc<AtomicBool>,
    ) -> Result<CommandResult, CommandError> {
        let started = Instant::now();
        let timeout = Duration::from_millis(config.timeout_ms);
        let (child, stdout, stderr) = start(&prepared)?;
        let limit = config.max_output_bytes;
        let out = thread::spawn(move || capture(stdout, limit));
        let err = thread::spawn(move || capture(stderr, limit));
        let mut cancelled = false;
        let mut timed_out = false;
        let mut exit_code = None;
        let mut wait_error = None;
        loop {
            match unsafe { WaitForSingleObject(child.process.as_raw_handle(), 0) } {
                WAIT_OBJECT_0 => {
                    let mut code = 0;
                    if unsafe { GetExitCodeProcess(child.process.as_raw_handle(), &mut code) } == 0
                    {
                        wait_error = Some(std::io::Error::last_os_error());
                    } else {
                        exit_code = Some(code);
                    }
                    break;
                }
                WAIT_TIMEOUT => {}
                _ => {
                    wait_error = Some(std::io::Error::last_os_error());
                    break;
                }
            }
            cancelled = cancellation.load(Ordering::Acquire);
            timed_out = started.elapsed() >= timeout;
            if cancelled || timed_out {
                break;
            }
            thread::sleep(
                Duration::from_millis(config.poll_interval_ms)
                    .min(timeout.saturating_sub(started.elapsed())),
            );
        }
        // Always close the private job before joining readers. Descendant pipes
        // must reach EOF even if the parent exits while a child is still alive.
        drop(child);
        let (stdout, stdout_truncated, stdout_lossy) = out
            .join()
            .map_err(|_| CommandError::Invalid("lector stdout falló".into()))??;
        let (stderr, stderr_truncated, stderr_lossy) = err
            .join()
            .map_err(|_| CommandError::Invalid("lector stderr falló".into()))??;
        if let Some(error) = wait_error {
            return Err(error.into());
        }
        Ok(CommandResult {
            id: prepared.spec.id,
            fingerprint: prepared.fingerprint,
            exit_code,
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
            stdout_lossy,
            stderr_lossy,
            timed_out,
            cancelled,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }
}
