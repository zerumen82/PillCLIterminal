//! Herramientas de texto con bases SHA-256 y publicación sin sobrescrituras.
//!
//! Windows: las carpetas permanecen abiertas sin compartir DELETE. La base se
//! bloquea contra escritura/borrado antes de calcular su hash. Una edición usa
//! dos renames por handle: base a backup, temporal a destino. Hay una ventana de
//! ausencia; un ocupante nuevo nunca se reemplaza. El backup se conserva.

use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const TRANSACTION_PREFIX: &str = ".pill-transaction-";
const CONTROL_COMPONENTS: &[&str] = &[".pillcli", ".git", ".agents", ".codex"];
const PRIVATE_NAMES: &[&str] = &["console.json", "model.json", "recovery.json"];
const JOURNAL_TAIL_LINES: usize = 16;
const JOURNAL_LINE_CHARS: usize = 240;
/// Tope de lectura del final del diario. No es una clave de configuración.
const JOURNAL_TAIL_BYTES: u64 = 16 * 1_048_576;
static TRANSACTION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FilesConfig {
    pub max_file_bytes: usize,
    pub max_entries: usize,
    pub max_search_results: usize,
    pub max_transactions: usize,
    pub excluded_components: Vec<String>,
    /// Propuestas visibles. No amplían la raíz de lectura ni de edición.
    #[serde(default)]
    pub extra_roots: Vec<PathBuf>,
}

impl FilesConfig {
    pub fn validate(&self) -> Result<(), FileError> {
        if !(1..=16_777_216).contains(&self.max_file_bytes)
            || !(1..=100_000).contains(&self.max_entries)
            || !(1..=10_000).contains(&self.max_search_results)
            || !(1..=10_000).contains(&self.max_transactions)
            || self.excluded_components.len() > 256
            || self.extra_roots.len() > 32
            || self.excluded_components.iter().any(|part| {
                part.is_empty()
                    || part.contains(['/', '\\', ':'])
                    || part.chars().any(char::is_control)
            })
        {
            return Err(FileError::Invalid("límites de files inválidos".into()));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("argumento de archivo inválido: {0}")]
    Invalid(String),
    #[error("ruta excluida o protegida: {0}")]
    Protected(String),
    #[error("conflicto: {0}")]
    Conflict(String),
    #[error("límite: {0}")]
    Limit(String),
    #[error("operación de archivo: {0}")]
    Io(#[from] std::io::Error),
    #[error("resultado parcial; destino {path}; base conservada en {backup_path}; {reason}")]
    Partial {
        path: String,
        backup_path: String,
        reason: String,
    },
    #[error("creación aplicada con verificación incompleta: {path}; {reason}")]
    Applied {
        path: String,
        sha256: String,
        reason: String,
    },
    #[error("{operation} aplicada con verificación incompleta en {path}: {reason}")]
    OperationPartial {
        operation: String,
        path: String,
        destination: Option<String>,
        reason: String,
    },
}

impl FileError {
    pub fn data(&self) -> Value {
        match self {
            Self::Partial {
                path,
                backup_path,
                reason,
            } => json!({
                "status":"partial", "path":path, "backup_path":backup_path,
                "reason":reason, "recovery":"No se restauró encima de un destino nuevo. Revise ambos archivos."
            }),
            Self::Applied {
                path,
                sha256,
                reason,
            } => json!({"status":"partial",
                "path":path,"intended_sha256":sha256,"reason":reason,
                "effect":"published","recovery":"Relea el destino; no repita la creación automáticamente."}),
            Self::OperationPartial {
                operation,
                path,
                destination,
                reason,
            } => json!({
                "status":"partial", "operation":operation, "path":path,
                "destination":destination, "reason":reason,
                "recovery":"Inspeccione las rutas; no repita la operación automáticamente."
            }),
            _ => json!({"status":"failed", "error":self.to_string()}),
        }
    }
}

fn summarize_journal_line(line: &str) -> String {
    let fallback = clip_journal(line);
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return fallback;
    };
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|item| !item.is_empty())
    };
    let parts: Vec<&str> = ["agent", "stage", "tool", "status", "path"]
        .into_iter()
        .filter_map(text)
        .collect();
    if parts.is_empty() {
        fallback
    } else {
        clip_journal(&parts.join(" · "))
    }
}

fn clip_journal(text: &str) -> String {
    text.chars()
        .take(JOURNAL_LINE_CHARS)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[derive(Debug)]
pub struct PrivateRead {
    pub bytes: Option<Vec<u8>>,
    pub recovered: bool,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactEdit {
    pub old: String,
    pub new: String,
}

/// Misma transformacion para intencion durable y publicacion, sobre base unica.
pub(crate) fn transform_exact(before: &[u8], edits: &[ExactEdit]) -> Result<Vec<u8>, FileError> {
    let (text, bom) = decode(before)?;
    let mut replacements = Vec::new();
    for edit in edits {
        if edit.old.is_empty() {
            return Err(FileError::Invalid("old no puede estar vacio".into()));
        }
        let start = text.find(&edit.old).ok_or_else(|| {
            FileError::Conflict("old no coincide; revisa texto y saltos CRLF/BOM".into())
        })?;
        let next = start
            + text[start..]
                .chars()
                .next()
                .expect("old no vacio")
                .len_utf8();
        if text[next..].contains(&edit.old) {
            return Err(FileError::Conflict("old coincide mas de una vez".into()));
        }
        replacements.push((start, start + edit.old.len(), edit.new.as_str()));
    }
    replacements.sort_by_key(|r| r.0);
    if replacements.windows(2).any(|w| w[0].1 > w[1].0) {
        return Err(FileError::Conflict("los parches se solapan".into()));
    }
    let mut after = text.to_owned();
    for (start, end, new) in replacements.into_iter().rev() {
        after.replace_range(start..end, new);
    }
    let mut bytes = Vec::new();
    if bom {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    bytes.extend_from_slice(after.as_bytes());
    Ok(bytes)
}

pub struct Files {
    root: PathBuf,
    config: FilesConfig,
    protected_paths: Vec<PathBuf>,
    // Mantiene estable la raíz y todos sus ancestros durante toda la sesión.
    _root_pins: Vec<File>,
    effects: Mutex<()>,
}

impl Files {
    pub fn proposed_roots(&self) -> &[PathBuf] {
        &self.config.extra_roots
    }

    pub fn new(root: &Path, config: FilesConfig) -> Result<Self, FileError> {
        config.validate()?;
        if !root.is_absolute() {
            return Err(FileError::Invalid("la raíz debe ser absoluta".into()));
        }
        let root = fs::canonicalize(root)?;
        let pins = platform::pin_chain(&root)?;
        if folded(&platform::final_path(pins.last().expect("raíz absoluta"))?) != folded(&root) {
            return Err(FileError::Conflict("la raíz cambió al abrirla".into()));
        }
        let executable = fs::canonicalize(std::env::current_exe()?)?;
        Ok(Self {
            root,
            config,
            protected_paths: vec![executable],
            _root_pins: pins,
            effects: Mutex::new(()),
        })
    }

    /// Las rutas protegidas vienen de composición de confianza, no del modelo.
    pub fn with_protected_paths(mut self, paths: Vec<PathBuf>) -> Result<Self, FileError> {
        for path in paths {
            if !path.is_absolute() {
                return Err(FileError::Invalid("protección debe ser absoluta".into()));
            }
            self.protected_paths.push(fs::canonicalize(path)?);
        }
        Ok(self)
    }

    /// Solo composición de confianza: el modelo jamás recibe esta capacidad.
    /// El llamador conserva los pins mientras use el diario privado.
    pub fn session_directory(&self) -> Result<(PathBuf, Vec<File>), FileError> {
        let mut path = self.root.clone();
        let mut pins = Vec::new();
        let mut parent = self._root_pins.last().expect("raíz fijada");
        for name in [".pillcli", "sessions"] {
            path.push(name);
            pins.push(platform::open_directory_at(parent, name.as_ref(), true)?);
            parent = pins.last().expect("directorio privado abierto");
        }
        Ok((path, pins))
    }

    /// Crea el diario privado directamente bajo su handle, sin reabrir la ruta.
    pub fn open_session_journal(
        &self,
        name: &str,
    ) -> Result<(PathBuf, Vec<File>, File), FileError> {
        if name.is_empty()
            || name.starts_with('.')
            || !name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
        {
            return Err(FileError::Invalid("nombre de diario inválido".into()));
        }
        let (directory, pins) = self.session_directory()?;
        let file =
            platform::create_file_at(pins.last().expect("directorio privado"), name.as_ref())?;
        Ok((directory.join(name), pins, file))
    }

    pub fn session_count(&self) -> Result<usize, FileError> {
        let (_directory, pins) = self.session_directory()?;
        let (entries, truncated) = platform::entries(
            pins.last().expect("directorio privado"),
            self.config.max_entries,
        )?;
        if truncated {
            return Err(FileError::Limit(
                "enumeración de sesiones supera files.max_entries".into(),
            ));
        }
        Ok(entries.len())
    }

    /// Lectura acotada de diarios mediante los mismos pins de la sesión. Una
    /// enumeración incompleta nunca se interpreta como recuperación completa.
    pub fn session_journals(
        &self,
        max_sessions: usize,
        max_journal_bytes: usize,
    ) -> Result<Vec<(String, Vec<u8>)>, FileError> {
        if max_sessions == 0 || max_journal_bytes == 0 {
            return Err(FileError::Invalid(
                "cotas de diarios deben ser positivas".into(),
            ));
        }
        let (_directory, pins) = self.session_directory()?;
        let directory = pins.last().expect("directorio privado");
        let (names, truncated) = platform::entries(directory, max_sessions)?;
        if truncated {
            return Err(FileError::Limit("diarios superan max_sessions".into()));
        }
        let mut journals = Vec::new();
        for name in names {
            let text = name
                .to_str()
                .ok_or_else(|| FileError::Invalid("nombre de diario no Unicode".into()))?;
            if !text.ends_with(".jsonl") {
                return Err(FileError::Invalid(format!(
                    "entrada inesperada en sesiones: {text}"
                )));
            }
            let mut file = platform::open_file_at(directory, &name, false)?;
            if file.metadata()?.len() > max_journal_bytes as u64 {
                return Err(FileError::Limit(format!(
                    "diario supera max_journal_bytes: {text}"
                )));
            }
            let mut bytes = Vec::new();
            (&mut file)
                .take(max_journal_bytes as u64 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > max_journal_bytes {
                return Err(FileError::Limit(format!(
                    "diario creció sobre la cota: {text}"
                )));
            }
            journals.push((text.to_owned(), bytes));
        }
        journals.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(journals)
    }

    /// Lectura privada con recuperación explícita de la última base JSON válida.
    pub fn read_private(&self, name: &str) -> Result<Option<Vec<u8>>, FileError> {
        Ok(self.read_private_checked(name)?.bytes)
    }

    pub fn read_private_checked(&self, name: &str) -> Result<PrivateRead, FileError> {
        self.read_private_checked_with(name, |bytes| serde_json::from_slice::<Value>(bytes).is_ok())
    }

    pub fn read_private_checked_with(
        &self,
        name: &str,
        valid: impl Fn(&[u8]) -> bool,
    ) -> Result<PrivateRead, FileError> {
        self.private_name(name)?;
        let directory = match self.private_directory(false) {
            Ok(directory) => directory,
            Err(FileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PrivateRead {
                    bytes: None,
                    recovered: false,
                    warning: None,
                });
            }
            Err(error) => return Err(error),
        };
        let mut primary_invalid = None;
        match platform::open_file_at(&directory, OsStr::new(name), false) {
            Ok(mut file) => {
                let bytes = Self::private_bytes(&mut file)?;
                if valid(&bytes) {
                    return Ok(PrivateRead {
                        bytes: Some(bytes),
                        recovered: false,
                        warning: None,
                    });
                }
                primary_invalid = Some("archivo principal JSON incompleto o inválido".to_owned());
            }
            Err(FileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let (names, truncated) = platform::entries(&directory, self.config.max_entries)?;
        if truncated {
            return Err(FileError::Limit(
                "no se puede enumerar completamente la recuperación privada".into(),
            ));
        }
        let mut best: Option<(u128, Vec<u8>)> = None;
        let mut unreadable_generation = false;
        for candidate in names {
            if !candidate.to_string_lossy().starts_with(TRANSACTION_PREFIX) {
                continue;
            }
            let transaction = platform::open_directory_at(&directory, &candidate, false)?;
            let Some((data, _)) = self.private_recovery_metadata(&transaction)? else {
                unreadable_generation = true;
                continue;
            };
            if data["operation"] != "private_write" || data["name"] != name {
                continue;
            }
            let Some(expected) = data["base_sha256"].as_str() else {
                continue;
            };
            let mut base = match platform::open_file_at(&transaction, "base".as_ref(), false) {
                Ok(file) => file,
                Err(FileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                    continue;
                }
                Err(error) => return Err(error),
            };
            let bytes = Self::private_bytes(&mut base)?;
            if hash(&bytes) != expected || !valid(&bytes) {
                continue;
            }
            let generation = data["generation_ns"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if best
                .as_ref()
                .is_none_or(|(previous, _)| generation > *previous)
            {
                best = Some((generation, bytes));
            }
        }
        if let Some((_, bytes)) = best {
            return Ok(PrivateRead {
                bytes: Some(bytes),
                recovered: true,
                warning: Some(format!(
                    "{name}: se recuperó el último respaldo válido; revisar el archivo principal"
                )),
            });
        }
        if let Some(error) = primary_invalid {
            return Err(FileError::Invalid(format!(
                "{name}: {error}; no existe respaldo válido"
            )));
        }
        Ok(PrivateRead {
            bytes: None,
            recovered: false,
            warning: unreadable_generation.then(||format!("{name}: hay generaciones privadas no validables; no se ha recuperado un historial")),
        })
    }

    fn private_bytes(file: &mut File) -> Result<Vec<u8>, FileError> {
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.take(JOURNAL_TAIL_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > JOURNAL_TAIL_BYTES {
            return Err(FileError::Limit("archivo privado demasiado grande".into()));
        }
        Ok(bytes)
    }

    fn private_recovery_metadata(
        &self,
        directory: &File,
    ) -> Result<Option<(Value, Option<String>)>, FileError> {
        let (data, information_sha) =
            match platform::open_file_at(directory, "base-info.json".as_ref(), false) {
                Ok(mut file) => {
                    let bytes = Self::private_bytes(&mut file)?;
                    let Ok(envelope) = serde_json::from_slice::<Value>(&bytes) else {
                        return Ok(None);
                    };
                    let Some(metadata) = envelope.get("metadata") else {
                        return Ok(None);
                    };
                    if envelope["sha256"].as_str()
                        != Some(hash(metadata.to_string().as_bytes()).as_str())
                        || metadata["metadata_version"] != 2
                    {
                        return Ok(None);
                    }
                    (metadata.clone(), Some(hash(&bytes)))
                }
                Err(FileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                    let mut file =
                        match platform::open_file_at(directory, "transaction.json".as_ref(), false)
                        {
                            Ok(file) => file,
                            Err(FileError::Io(error))
                                if error.kind() == std::io::ErrorKind::NotFound =>
                            {
                                return Ok(None);
                            }
                            Err(error) => return Err(error),
                        };
                    let Ok(data) =
                        serde_json::from_slice::<Value>(&Self::private_bytes(&mut file)?)
                    else {
                        return Ok(None);
                    };
                    // Las generaciones nuevas no degradan a metadatos mutables si
                    // falta su sobre inmutable. Solo lectura heredada explícita.
                    if data["metadata_version"] == 2 {
                        return Ok(None);
                    }
                    (data, None)
                }
                Err(error) => return Err(error),
            };
        let Some(name) = data["name"].as_str() else {
            return Ok(None);
        };
        let valid_hash = |value: &Value| {
            value
                .as_str()
                .is_some_and(|sha| sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        };
        if data["operation"] != "private_write"
            || !PRIVATE_NAMES.contains(&name)
            || !valid_hash(&data["intended_sha256"])
            || (!data["base_sha256"].is_null() && !valid_hash(&data["base_sha256"]))
            || data["path"].as_str().is_none_or(|path| {
                folded(Path::new(path)) != folded(&self.root.join(".pillcli").join(name))
            })
        {
            return Ok(None);
        }
        Ok(Some((data, information_sha)))
    }

    fn stable_private_metadata(value: &Value) -> Value {
        let mut result = value.clone();
        if let Some(map) = result.as_object_mut() {
            map.remove("state");
            map.remove("base_info_sha256");
        }
        result
    }

    /// Conserva la base y publica sin reemplazar un ocupante concurrente.
    pub fn write_private(&self, name: &str, bytes: &[u8]) -> Result<(), FileError> {
        self.private_name(name)?;
        if bytes.len() as u64 > JOURNAL_TAIL_BYTES {
            return Err(FileError::Limit("archivo privado demasiado grande".into()));
        }
        serde_json::from_slice::<Value>(bytes)
            .map_err(|e| FileError::Invalid(format!("JSON privado inválido: {e}")))?;
        let _effect = self
            .effects
            .lock()
            .map_err(|_| FileError::Conflict("bloqueo interno".into()))?;
        let directory = self.private_directory(true)?;
        let _lock = platform::private_lock_at(&directory)?;
        let mut original = match platform::open_file_at(&directory, name.as_ref(), true) {
            Ok(file) => Some(file),
            Err(FileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let before = original.as_mut().map(Self::private_bytes).transpose()?;
        if before.as_deref() == Some(bytes) {
            return Ok(());
        }
        let transaction = self.transaction(&directory, &self.root.join(".pillcli"))?;
        let mut staged = platform::create_file_at(&transaction.pin, "new".as_ref())?;
        if let Some(original) = &original {
            platform::copy_security(original, &staged)?;
        }
        staged.write_all(bytes)?;
        staged.sync_all()?;
        if Self::private_bytes(&mut staged)? != bytes {
            return Err(FileError::Conflict("temporal privado no coincide".into()));
        }
        let mut data = json!({"metadata_version":2,"operation":"private_write","name":name,"path":self.root.join(".pillcli").join(name),"base_sha256":before.as_ref().map(|b|hash(b)),"intended_sha256":hash(bytes),"generation_ns":SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e|FileError::Invalid(e.to_string()))?.as_nanos().to_string(),"state":"prepared"});
        // El sobre de recuperación se escribe y sincroniza una sola vez ANTES
        // de mover la base. Las actualizaciones del estado nunca lo truncan.
        let information_bytes = serde_json::to_vec(
            &json!({"metadata":data,"sha256":hash(data.to_string().as_bytes())}),
        )
        .map_err(|e| FileError::Invalid(e.to_string()))?;
        let mut information =
            platform::create_file_at(&transaction.pin, "base-info.json".as_ref())?;
        information.write_all(&information_bytes)?;
        information.sync_all()?;
        if Self::private_bytes(&mut information)? != information_bytes {
            return Err(FileError::Conflict(
                "metadatos de recuperación privados no coinciden".into(),
            ));
        }
        data["base_info_sha256"] = json!(hash(&information_bytes));
        let mut manifest = platform::create_file_at(&transaction.pin, "transaction.json".as_ref())?;
        write_manifest(&mut manifest, &data)?;
        if let Some(original) = &original {
            platform::rename_at(original, &transaction.pin, "base".as_ref())?;
            data["state"] = json!("base_moved");
            write_manifest(&mut manifest, &data).map_err(|error| FileError::Partial {
                path: format!(".pillcli/{name}"),
                backup_path: transaction.path.join("base").display().to_string(),
                reason: error.to_string(),
            })?;
            publish_staged(
                original,
                &staged,
                &directory,
                name.as_ref(),
                &transaction.path.join("base"),
                &format!(".pillcli/{name}"),
            )?;
        } else {
            platform::rename_at(&staged, &directory, name.as_ref()).map_err(|e| {
                FileError::Conflict(format!("publicación privada no aplicada: {e}"))
            })?;
        }
        let expected = self.root.join(".pillcli").join(name);
        if Self::private_bytes(&mut staged)? != bytes
            || folded(&platform::final_path(&staged)?) != folded(&expected)
        {
            return Err(FileError::Partial {
                path: format!(".pillcli/{name}"),
                backup_path: transaction.path.join("base").display().to_string(),
                reason: "publicación privada no confirmada".into(),
            });
        }
        data["state"] = json!("verified");
        write_manifest(&mut manifest, &data).map_err(|e| FileError::Partial {
            path: format!(".pillcli/{name}"),
            backup_path: transaction.path.join("base").display().to_string(),
            reason: e.to_string(),
        })?;
        Ok(())
    }

    /// Últimas líneas del diario ya existente. No crea una sesión nueva.
    pub fn journal_tail(&self) -> Result<String, FileError> {
        let root = self._root_pins.last().expect("raíz fijada");
        let private = match platform::open_directory_at(root, OsStr::new(".pillcli"), false) {
            Ok(directory) => directory,
            Err(FileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok("No hay diario en este proyecto.".into());
            }
            Err(error) => return Err(error),
        };
        let sessions = match platform::open_directory_at(&private, OsStr::new("sessions"), false) {
            Ok(directory) => directory,
            Err(FileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok("No hay diario en este proyecto.".into());
            }
            Err(error) => return Err(error),
        };
        let (names, truncated) = platform::entries(&sessions, self.config.max_entries)?;
        if truncated {
            return Err(FileError::Limit(
                "enumeración de sesiones supera files.max_entries".into(),
            ));
        }
        let mut newest: Option<(SystemTime, OsString)> = None;
        for name in names {
            if !name.to_string_lossy().ends_with(".jsonl") {
                continue;
            }
            let file = platform::open_file_at(&sessions, &name, false)?;
            let modified = file.metadata()?.modified().unwrap_or(UNIX_EPOCH);
            if newest.as_ref().is_none_or(|(time, _)| modified >= *time) {
                newest = Some((modified, name));
            }
        }
        let Some((_, name)) = newest else {
            return Ok("No hay diario en este proyecto.".into());
        };
        let mut file = platform::open_file_at(&sessions, &name, false)?;
        let length = file.metadata()?.len();
        if length > JOURNAL_TAIL_BYTES {
            file.seek(SeekFrom::End(-(JOURNAL_TAIL_BYTES as i64)))?;
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let text = String::from_utf8_lossy(&bytes);
        let mut lines: Vec<&str> = text.lines().collect();
        if length > JOURNAL_TAIL_BYTES {
            lines = lines.split_off(1.min(lines.len()));
        }
        let tail: Vec<String> = lines
            .into_iter()
            .rev()
            .filter(|line| !line.trim().is_empty())
            .take(JOURNAL_TAIL_LINES)
            .map(summarize_journal_line)
            .collect();
        if tail.is_empty() {
            return Ok(format!("Diario {} · vacío", name.to_string_lossy()));
        }
        let mut shown = tail;
        shown.reverse();
        Ok(format!(
            "Diario {}\n{}",
            name.to_string_lossy(),
            shown.join("\n")
        ))
    }

    fn private_name(&self, name: &str) -> Result<(), FileError> {
        if PRIVATE_NAMES.contains(&name) {
            Ok(())
        } else {
            Err(FileError::Protected("nombre privado no admitido".into()))
        }
    }

    fn private_directory(&self, create: bool) -> Result<File, FileError> {
        platform::open_directory_at(
            self._root_pins.last().expect("raíz fijada"),
            OsStr::new(".pillcli"),
            create,
        )
    }

    pub fn read(&self, path: &str) -> Result<Value, FileError> {
        let resolved = self.resolve(path, false)?;
        let mut file = platform::open_file_at(&resolved.parent, &resolved.leaf, false)?;
        self.check_handle(&file)?;
        let bytes = self.read_bytes(&mut file)?;
        let (content, bom) = decode(&bytes)?;
        Ok(json!({"status":"verified", "path":path, "content":content,
            "sha256":hash(&bytes), "bytes":bytes.len(), "utf8_bom":bom,
            "truncated":false, "line_endings":line_endings(content)}))
    }

    /// Listado no recursivo; search recorre directorios con la misma política.
    pub fn list(&self, path: &str) -> Result<Value, FileError> {
        let resolved = self.resolve(path, true)?;
        let mut entries = Vec::new();
        let mut omitted = 0usize;
        let directory = resolved
            .entry
            .as_ref()
            .ok_or_else(|| FileError::Invalid("se requiere directorio".into()))?;
        let (children, truncated) = platform::entries(directory, self.config.max_entries)?;
        for name in children {
            let child = self.relative(&resolved.path.join(name))?;
            match self.resolve(&child, true) {
                Ok(checked) => {
                    let file = checked.entry.as_ref().expect("inspección conserva handle");
                    let metadata = file.metadata()?;
                    entries.push(json!({"path":child,
                        "kind":if metadata.is_dir() { "directory" } else { "file" },
                        "bytes":metadata.len()}));
                }
                Err(_) => omitted += 1,
            }
        }
        Ok(json!({"status":"verified", "path":path, "entries":entries,
            "omitted":omitted, "truncated":truncated}))
    }

    pub fn search(&self, path: &str, needle: &str) -> Result<Value, FileError> {
        if needle.is_empty() || needle.len() > self.config.max_file_bytes {
            return Err(FileError::Invalid(
                "búsqueda vacía o demasiado grande".into(),
            ));
        }
        let initial = self.resolve(path, true)?;
        let mut pending = vec![initial.path.clone()];
        let mut matches = Vec::new();
        let mut scanned = 0usize;
        let mut omitted = 0usize;
        let mut truncated = false;
        let mut output_bytes = 0usize;
        'walk: while let Some(current) = pending.pop() {
            if scanned >= self.config.max_entries {
                truncated = true;
                break;
            }
            scanned += 1;
            let relative = self.relative(&current)?;
            let checked = match self.resolve(&relative, true) {
                Ok(checked) => checked,
                Err(_) => {
                    omitted += 1;
                    continue;
                }
            };
            if checked
                .entry
                .as_ref()
                .is_some_and(|entry| platform::is_directory(entry).unwrap_or(false))
            {
                // Acotar también la cola: un directorio enorme no consume memoria ilimitada.
                let (children, partial) = platform::entries(
                    checked.entry.as_ref().expect("directorio"),
                    self.config
                        .max_entries
                        .saturating_sub(scanned + pending.len()),
                )?;
                truncated |= partial;
                pending.extend(children.into_iter().map(|name| checked.path.join(name)));
                continue;
            }
            let read = match self.read(&relative) {
                Ok(read) => read,
                Err(_) => {
                    omitted += 1;
                    continue;
                }
            };
            let content = read["content"].as_str().expect("read devuelve texto");
            for (line, text) in content.lines().enumerate() {
                if text.contains(needle) {
                    if matches.len() == self.config.max_search_results {
                        truncated = true;
                        break 'walk;
                    }
                    let candidate = json!({"path":relative,"line":line+1,"text":text,
                        "sha256":read["sha256"]});
                    let bytes = candidate.to_string().len();
                    // El mismo presupuesto configurable acota archivo y suma
                    // de resultados serializados, incluidos rutas y hashes.
                    if output_bytes.saturating_add(bytes) > self.config.max_file_bytes {
                        truncated = true;
                        break 'walk;
                    }
                    output_bytes += bytes;
                    matches.push(candidate);
                }
            }
        }
        Ok(json!({"status":"verified", "path":path, "matches":matches,
            "scanned":scanned,"omitted":omitted,"truncated":truncated,"result_bytes":output_bytes,
            "exhaustive":!truncated && omitted == 0}))
    }

    /// Metadatos y versión de un destino concreto, sin conceder operaciones.
    pub fn inspect_operation(&self, path: &str) -> Result<Value, FileError> {
        let snapshot = self.operation_snapshot(path, false)?;
        Ok(
            json!({"status":"verified", "path":path, "kind":snapshot.kind,
            "sha256":snapshot.sha256, "bytes":snapshot.bytes,
            "identity":snapshot.identity, "utf8_text":snapshot.utf8_text}),
        )
    }

    /// La huella solo es una propuesta. La composición de confianza debe
    /// mostrarla y obtener el consentimiento explícito antes de entregarla a
    /// move_file/delete; el modelo nunca concede su propia propuesta.
    pub fn operation_offer(
        &self,
        operation: &str,
        path: &str,
        destination: Option<&str>,
        expected_sha256: &str,
    ) -> Result<Value, FileError> {
        let snapshot = self.operation_snapshot(path, false)?;
        self.operation_details(operation, path, destination, expected_sha256, &snapshot)
    }

    pub fn move_file(
        &self,
        path: &str,
        destination: &str,
        expected_sha256: &str,
        approved_fingerprint: &str,
    ) -> Result<Value, FileError> {
        let _effect = self
            .effects
            .lock()
            .map_err(|_| FileError::Conflict("bloqueo interno".into()))?;
        let mut snapshot = self.operation_snapshot(path, true)?;
        let offer =
            self.operation_details("move", path, Some(destination), expected_sha256, &snapshot)?;
        Self::check_operation_approval(&offer, approved_fingerprint)?;
        let target = self.resolve(destination, false)?;
        let partial = |reason: String| FileError::OperationPartial {
            operation: "move".into(),
            path: path.into(),
            destination: Some(destination.into()),
            reason,
        };
        platform::rename_at(&snapshot.file, &target.parent, &target.leaf).map_err(|error| {
            FileError::Conflict(format!("movimiento sin sobrescribir rechazado: {error}"))
        })?;
        let actual = self
            .operation_bytes(&mut snapshot.file)
            .map_err(|error| partial(error.to_string()))?;
        if hash(&actual) != snapshot.sha256 {
            return Err(partial("bytes distintos tras mover".into()));
        }
        if folded(
            &platform::final_path(&snapshot.file).map_err(|error| partial(error.to_string()))?,
        ) != folded(&target.path)
        {
            return Err(partial("nombre final distinto del destino".into()));
        }
        Self::verify_absent(&snapshot.resolved).map_err(|error| partial(error.to_string()))?;
        Ok(json!({"status":"verified", "operation":"move", "path":path,
            "destination":destination, "sha256":snapshot.sha256,
            "base_sha256":snapshot.sha256, "bytes":actual.len(), "source_absent":true}))
    }

    pub fn delete(
        &self,
        path: &str,
        expected_sha256: &str,
        approved_fingerprint: &str,
    ) -> Result<Value, FileError> {
        let _effect = self
            .effects
            .lock()
            .map_err(|_| FileError::Conflict("bloqueo interno".into()))?;
        let snapshot = self.operation_snapshot(path, true)?;
        let offer = self.operation_details("delete", path, None, expected_sha256, &snapshot)?;
        Self::check_operation_approval(&offer, approved_fingerprint)?;
        platform::mark_delete(&snapshot.file)?;
        drop(snapshot.file);
        Self::verify_absent(&snapshot.resolved).map_err(|error| FileError::OperationPartial {
            operation: "delete".into(),
            path: path.into(),
            destination: None,
            reason: error.to_string(),
        })?;
        Ok(
            json!({"status":"verified", "operation":"delete", "path":path,
            "sha256":snapshot.sha256, "base_sha256":snapshot.sha256,
            "kind":snapshot.kind, "absent":true}),
        )
    }

    fn check_operation_approval(offer: &Value, approved: &str) -> Result<(), FileError> {
        if approved.is_empty() || offer["fingerprint"].as_str() != Some(approved) {
            return Err(FileError::Protected(
                "falta confirmación explícita de esta operación y versión".into(),
            ));
        }
        Ok(())
    }

    fn operation_snapshot(&self, path: &str, mutate: bool) -> Result<OperationSnapshot, FileError> {
        let mut resolved = self.resolve(path, false)?;
        let mut file = platform::open_operation_at(&resolved.parent, &resolved.leaf, mutate)?;
        resolved.path = platform::final_path(&file)?;
        self.check_path(&resolved.path)?;
        let identity = platform::identity(&file)?;
        let (kind, sha256, bytes, utf8_text) = if platform::is_directory(&file)? {
            if !platform::empty_directory(&file)? {
                return Err(FileError::Conflict(
                    "solo se admiten carpetas vacías".into(),
                ));
            }
            (
                "empty_directory",
                hash(format!("pill-empty-directory-v1\n{identity}").as_bytes()),
                0,
                false,
            )
        } else {
            self.check_handle(&file)?;
            let bytes = self.operation_bytes(&mut file)?;
            ("file", hash(&bytes), bytes.len(), decode(&bytes).is_ok())
        };
        Ok(OperationSnapshot {
            resolved,
            file,
            kind,
            sha256,
            bytes,
            identity,
            utf8_text,
        })
    }

    fn operation_bytes(&self, file: &mut File) -> Result<Vec<u8>, FileError> {
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.take(self.config.max_file_bytes as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > self.config.max_file_bytes {
            return Err(FileError::Limit("archivo supera max_file_bytes".into()));
        }
        Ok(bytes)
    }

    fn operation_details(
        &self,
        operation: &str,
        path: &str,
        destination: Option<&str>,
        expected_sha256: &str,
        snapshot: &OperationSnapshot,
    ) -> Result<Value, FileError> {
        if expected_sha256.len() != 64
            || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(FileError::Invalid(
                "sha256 debe tener 64 dígitos hexadecimales".into(),
            ));
        }
        if !snapshot.sha256.eq_ignore_ascii_case(expected_sha256) {
            return Err(FileError::Conflict(format!(
                "la base de {path} cambió; SHA actual {}",
                snapshot.sha256
            )));
        }
        let destination_path = match (operation, destination) {
            ("move", Some(destination)) if snapshot.kind == "file" => {
                let target = self.resolve(destination, false)?;
                if folded(&target.path) == folded(&snapshot.resolved.path) {
                    return Err(FileError::Conflict(
                        "origen y destino son el mismo archivo".into(),
                    ));
                }
                Self::verify_absent(&target)?;
                Some(folded(&target.path))
            }
            ("delete", None) => None,
            _ => {
                return Err(FileError::Invalid(
                    "operación o destino no admitidos".into(),
                ));
            }
        };
        let mut detail = json!({"operation":operation, "root":folded(&self.root),
            "path":path, "source_path":folded(&snapshot.resolved.path),
            "destination":destination, "destination_path":destination_path,
            "sha256":snapshot.sha256, "identity":snapshot.identity, "kind":snapshot.kind,
            "bytes":snapshot.bytes});
        detail["fingerprint"] = json!(hash(detail.to_string().as_bytes()));
        Ok(detail)
    }

    fn verify_absent(resolved: &Resolved) -> Result<(), FileError> {
        match platform::open_any_at(&resolved.parent, &resolved.leaf) {
            Err(FileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(FileError::Conflict("la ruta está ocupada".into())),
            Err(error) => Err(error),
        }
    }

    pub fn create(&self, path: &str, content: &str) -> Result<Value, FileError> {
        let _effect = self
            .effects
            .lock()
            .map_err(|_| FileError::Conflict("bloqueo interno".into()))?;
        self.validate_text(content.as_bytes())?;
        let resolved = self.resolve(path, false)?;
        let parent = resolved
            .path
            .parent()
            .ok_or_else(|| FileError::Invalid(path.into()))?;
        let transaction = self.transaction(&resolved.parent, parent)?;
        let mut staged = platform::create_file_at(&transaction.pin, "new".as_ref())?;
        staged.write_all(content.as_bytes())?;
        staged.sync_all()?;
        platform::rename_at(&staged, &resolved.parent, &resolved.leaf).map_err(|error| {
            FileError::Conflict(format!("no se creó {path} sin sobrescribir: {error}"))
        })?;
        let actual = self
            .read_bytes(&mut staged)
            .map_err(|error| FileError::Applied {
                path: path.into(),
                sha256: hash(content.as_bytes()),
                reason: error.to_string(),
            })?;
        if actual != content.as_bytes() {
            // El handle publicado sigue bloqueado; no inventar verificación.
            return Err(FileError::Applied {
                path: path.into(),
                sha256: hash(content.as_bytes()),
                reason: "relectura de creación diferente".into(),
            });
        }
        if folded(
            &platform::final_path(&staged).map_err(|error| FileError::Applied {
                path: path.into(),
                sha256: hash(&actual),
                reason: error.to_string(),
            })?,
        ) != folded(&resolved.path)
        {
            return Err(FileError::Applied {
                path: path.into(),
                sha256: hash(&actual),
                reason: "el nombre final no coincide con el destino".into(),
            });
        }
        drop(staged);
        Ok(
            json!({"status":"verified","path":path,"sha256":hash(&actual),
            "bytes":actual.len(),"transaction_path":transaction.path,"diff":{"before":null,"after":content}}),
        )
    }

    pub fn edit(
        &self,
        path: &str,
        expected_sha256: &str,
        old: &str,
        new: &str,
    ) -> Result<Value, FileError> {
        self.edit_many(
            path,
            expected_sha256,
            &[ExactEdit {
                old: old.into(),
                new: new.into(),
            }],
        )
    }

    pub fn edit_many(
        &self,
        path: &str,
        expected_sha256: &str,
        edits: &[ExactEdit],
    ) -> Result<Value, FileError> {
        if edits.is_empty() || edits.len() > self.config.max_entries {
            return Err(FileError::Limit("número de parches inválido".into()));
        }
        self.replace_text_bytes(path, expected_sha256, "edit", |before| {
            transform_exact(before, edits)
        })
    }

    fn replace_text_bytes(
        &self,
        path: &str,
        expected_sha256: &str,
        operation: &str,
        transform: impl FnOnce(&[u8]) -> Result<Vec<u8>, FileError>,
    ) -> Result<Value, FileError> {
        let _effect = self
            .effects
            .lock()
            .map_err(|_| FileError::Conflict("bloqueo interno".into()))?;
        if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(FileError::Invalid(
                "sha256 debe tener 64 dígitos hexadecimales".into(),
            ));
        }
        let mut resolved = self.resolve(path, false)?;
        let mut original = platform::open_file_at(&resolved.parent, &resolved.leaf, true)?;
        self.check_handle(&original)?;
        resolved.path = platform::final_path(&original)?;
        resolved.leaf = resolved
            .path
            .file_name()
            .ok_or_else(|| FileError::Invalid("archivo sin nombre".into()))?
            .to_os_string();
        let before = self.read_bytes(&mut original)?;
        let base_sha = hash(&before);
        if !base_sha.eq_ignore_ascii_case(expected_sha256) {
            return Err(FileError::Conflict(format!(
                "la base de {path} cambió; SHA actual {base_sha}"
            )));
        }
        let (text, bom) = decode(&before)?;
        let after = transform(&before)?;
        self.validate_text(&after)?;
        let (after_text, _) = decode(&after)?;
        let parent = resolved
            .path
            .parent()
            .ok_or_else(|| FileError::Invalid(path.into()))?;
        let transaction = self.transaction(&resolved.parent, parent)?;
        let backup = transaction.path.join("base");
        let mut staged = platform::create_file_at(&transaction.pin, "new".as_ref())?;
        platform::copy_security(&original, &staged)?;
        staged.write_all(&after)?;
        staged.sync_all()?;
        let mut manifest = platform::create_file_at(&transaction.pin, "transaction.json".as_ref())?;
        let mut manifest_data = json!({"operation":operation,"path":resolved.path,
            "base_sha256":base_sha,"intended_sha256":hash(&after),
            "backup_path":backup,"state":"prepared"});
        write_manifest(&mut manifest, &manifest_data)?;
        platform::rename_at(&original, &transaction.pin, "base".as_ref())?;
        manifest_data["state"] = json!("base_moved");
        write_manifest(&mut manifest, &manifest_data).map_err(|error| FileError::Partial {
            path: path.into(),
            backup_path: backup.display().to_string(),
            reason: error.to_string(),
        })?;
        if let Err(error) = publish_staged(
            &original,
            &staged,
            &resolved.parent,
            &resolved.leaf,
            &backup,
            path,
        ) {
            manifest_data["state"] = json!(match error {
                FileError::Partial { .. } => "partial",
                _ => "restored",
            });
            manifest_data["error"] = json!(error.to_string());
            let _ = write_manifest(&mut manifest, &manifest_data);
            return Err(error);
        }
        let actual = self
            .read_bytes(&mut staged)
            .map_err(|error| FileError::Partial {
                path: path.into(),
                backup_path: backup.display().to_string(),
                reason: error.to_string(),
            })?;
        if actual != after {
            return Err(FileError::Partial {
                path: path.into(),
                backup_path: backup.display().to_string(),
                reason: "la relectura publicada no coincide".into(),
            });
        }
        let final_path = platform::final_path(&staged).map_err(|error| FileError::Partial {
            path: path.into(),
            backup_path: backup.display().to_string(),
            reason: error.to_string(),
        })?;
        if folded(&final_path) != folded(&resolved.path) {
            return Err(FileError::Partial {
                path: path.into(),
                backup_path: backup.display().to_string(),
                reason: "el nombre final no coincide con el destino".into(),
            });
        }
        manifest_data["state"] = json!("verified");
        write_manifest(&mut manifest, &manifest_data).map_err(|error| FileError::Partial {
            path: path.into(),
            backup_path: backup.display().to_string(),
            reason: error.to_string(),
        })?;
        Ok(
            json!({"status":"verified","path":path,"sha256":hash(&actual),
            "base_sha256":base_sha,"bytes":actual.len(),"backup_path":backup,
            "backup_sha256":hash(&before),"publication":"two_no_replace_renames",
            "security":"owner_group_dacl_copied","transaction_path":transaction.path,
            "diff":{"before":text,"after":after_text},"utf8_bom":bom,"operation":operation}),
        )
    }

    /// Solo composición humana: revierte una edición cuya versión posterior sigue vigente.
    pub fn restore_backup(
        &self,
        path: &str,
        expected_sha256: &str,
        backup_path: &str,
        backup_sha256: &str,
    ) -> Result<Value, FileError> {
        let resolved = self.resolve(path, false)?;
        let supplied = Path::new(backup_path);
        if !supplied.is_absolute()
            || supplied
                .components()
                .any(|c| matches!(c, Component::ParentDir))
            || supplied.file_name() != Some(OsStr::new("base"))
        {
            return Err(FileError::Protected(
                "respaldo debe ser base de una transacción propia".into(),
            ));
        }
        let directory_path = supplied
            .parent()
            .ok_or_else(|| FileError::Invalid("respaldo sin carpeta".into()))?;
        let tx_name = directory_path
            .file_name()
            .ok_or_else(|| FileError::Invalid("transacción sin nombre".into()))?;
        if !tx_name.to_string_lossy().starts_with(TRANSACTION_PREFIX)
            || directory_path
                .parent()
                .is_none_or(|p| folded(p) != folded(resolved.path.parent().expect("resuelto")))
        {
            return Err(FileError::Protected(
                "respaldo fuera del padre de la edición".into(),
            ));
        }
        let directory = match platform::open_directory_at(&resolved.parent, tx_name, false) {
            Ok(directory) => directory,
            Err(FileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                let relative = self.relative(directory_path)?;
                let locator = Self::transaction_locator(&relative);
                let private = self.private_directory(false)?;
                let archive = platform::open_directory_at(&private, "archive".as_ref(), false)?;
                let transactions =
                    platform::open_directory_at(&archive, "transactions".as_ref(), false)?;
                platform::open_directory_at(&transactions, locator.as_ref(), false)?
            }
            Err(error) => return Err(error),
        };
        let mut manifest = platform::open_file_at(&directory, "transaction.json".as_ref(), false)?;
        let data: Value = serde_json::from_slice(&Self::private_bytes(&mut manifest)?)
            .map_err(|e| FileError::Invalid(e.to_string()))?;
        if !matches!(data["operation"].as_str(), Some("edit" | "restore"))
            || data["state"] != "verified"
            || data["path"]
                .as_str()
                .is_none_or(|p| folded(Path::new(p)) != folded(&resolved.path))
            || data["base_sha256"]
                .as_str()
                .is_none_or(|sha| !sha.eq_ignore_ascii_case(backup_sha256))
            || data["intended_sha256"]
                .as_str()
                .is_none_or(|sha| !sha.eq_ignore_ascii_case(expected_sha256))
        {
            return Err(FileError::Conflict(
                "el respaldo no corresponde a la edición y versión solicitadas".into(),
            ));
        }
        let mut backup = platform::open_file_at(&directory, "base".as_ref(), false)?;
        let bytes = self.read_bytes(&mut backup)?;
        if !hash(&bytes).eq_ignore_ascii_case(backup_sha256) {
            return Err(FileError::Conflict("el respaldo cambió".into()));
        }
        let mut result =
            self.replace_text_bytes(path, expected_sha256, "restore", |_| Ok(bytes))?;
        result["restored_from"] = json!(backup_path);
        Ok(result)
    }

    pub fn retention_status(&self) -> Result<Value, FileError> {
        let private = self.private_directory(true)?;
        let (names, truncated) = platform::entries(&private, self.config.max_entries)?;
        if truncated {
            return Err(FileError::Limit(
                "cuota privada no enumerable por completo".into(),
            ));
        }
        let private_transactions = names
            .iter()
            .filter(|n| n.to_string_lossy().starts_with(TRANSACTION_PREFIX))
            .count();
        let root = self._root_pins.last().expect("raíz fijada");
        let (names, truncated) = platform::entries(root, self.config.max_entries)?;
        if truncated {
            return Err(FileError::Limit(
                "cuota raíz no enumerable por completo".into(),
            ));
        }
        let root_transactions = names
            .iter()
            .filter(|n| n.to_string_lossy().starts_with(TRANSACTION_PREFIX))
            .count();
        Ok(
            json!({"private_versions":private_transactions,"private_versions_limit":self.config.max_transactions,"root_backups":root_transactions,"root_backups_limit":self.config.max_transactions,"sessions":self.session_count()?,"archive":".pillcli/archive","warning":if private_transactions>=self.config.max_transactions || root_transactions>=self.config.max_transactions {Some("cuota agotada; revisar y archivar evidencia antes de continuar")} else {None}}),
        )
    }

    /// Mueve solo el diario concreto cuya huella se mostró al usuario; no borra bytes.
    pub fn archive_session(&self, name: &str, expected_sha256: &str) -> Result<Value, FileError> {
        if !name.ends_with(".jsonl")
            || name.starts_with('.')
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(FileError::Invalid("nombre de diario inválido".into()));
        }
        let (_path, pins) = self.session_directory()?;
        let directory = pins.last().expect("sesiones fijadas");
        let mut file = platform::open_file_at(directory, name.as_ref(), true)?;
        let bytes = Self::private_bytes(&mut file)?;
        if !hash(&bytes).eq_ignore_ascii_case(expected_sha256) {
            return Err(FileError::Conflict(
                "el diario cambió; vuelve a mostrar el archivo".into(),
            ));
        }
        let private = self.private_directory(true)?;
        let archive = platform::open_directory_at(&private, "archive".as_ref(), true)?;
        let archived_sessions = platform::open_directory_at(&archive, "sessions".as_ref(), true)?;
        platform::rename_at(&file, &archived_sessions, name.as_ref())?;
        let after = Self::private_bytes(&mut file)?;
        if hash(&after) != hash(&bytes)
            || folded(&platform::final_path(&file)?)
                != folded(&self.root.join(".pillcli/archive/sessions").join(name))
        {
            return Err(FileError::OperationPartial {
                operation: "archive".into(),
                path: format!(".pillcli/sessions/{name}"),
                destination: Some(format!(".pillcli/archive/sessions/{name}")),
                reason: "archivo movido sin confirmación completa".into(),
            });
        }
        Ok(
            json!({"status":"verified","journal":name,"sha256":hash(&after),"destination":format!(".pillcli/archive/sessions/{name}")}),
        )
    }

    /// Versiones privadas cerradas que no son el último respaldo válido de su archivo.
    pub fn private_archive_offers(&self) -> Result<Vec<Value>, FileError> {
        self.private_archive_offers_with(|_, bytes| serde_json::from_slice::<Value>(bytes).is_ok())
    }

    pub fn private_archive_offers_with(
        &self,
        valid: impl Fn(&str, &[u8]) -> bool,
    ) -> Result<Vec<Value>, FileError> {
        let directory = self.private_directory(true)?;
        let (names, truncated) = platform::entries(&directory, self.config.max_entries)?;
        if truncated {
            return Err(FileError::Limit(
                "versiones privadas no enumerables por completo".into(),
            ));
        }
        let mut candidates = Vec::new();
        let mut last_valid: std::collections::BTreeMap<String, (u128, String)> =
            std::collections::BTreeMap::new();
        for name in names {
            if !name.to_string_lossy().starts_with(TRANSACTION_PREFIX) {
                continue;
            }
            let transaction = platform::open_directory_at(&directory, &name, false)?;
            let (members, truncated) = platform::entries(&transaction, self.config.max_entries)?;
            if truncated
                || members.iter().any(|n| {
                    n != OsStr::new("base")
                        && n != OsStr::new("transaction.json")
                        && n != OsStr::new("base-info.json")
                })
            {
                continue;
            }
            let mut manifest =
                match platform::open_file_at(&transaction, "transaction.json".as_ref(), false) {
                    Ok(file) => file,
                    Err(FileError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e),
                };
            let manifest_bytes = Self::private_bytes(&mut manifest)?;
            let Ok(data) = serde_json::from_slice::<Value>(&manifest_bytes) else {
                continue;
            };
            let Some((recovery_data, information_sha)) =
                self.private_recovery_metadata(&transaction)?
            else {
                continue;
            };
            if Self::stable_private_metadata(&data) != Self::stable_private_metadata(&recovery_data)
                || data.get("base_info_sha256").and_then(Value::as_str)
                    != information_sha.as_deref()
            {
                continue;
            }
            let Some(target) = data["name"].as_str() else {
                continue;
            };
            if data["operation"] != "private_write"
                || data["state"] != "verified"
                || !PRIVATE_NAMES.contains(&target)
            {
                continue;
            }
            let generation = data["generation_ns"]
                .as_str()
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);
            let mut base_valid = false;
            let base_hash = if let Some(expected) = data["base_sha256"].as_str() {
                let mut base = match platform::open_file_at(&transaction, "base".as_ref(), false) {
                    Ok(file) => file,
                    Err(FileError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e),
                };
                let bytes = Self::private_bytes(&mut base)?;
                if hash(&bytes) != expected {
                    continue;
                }
                base_valid = valid(target, &bytes);
                Some(hash(&bytes))
            } else {
                if members.iter().any(|n| n == OsStr::new("base")) {
                    continue;
                }
                None
            };
            let name = name.to_string_lossy().into_owned();
            if base_valid
                && last_valid
                    .get(target)
                    .is_none_or(|(old, _)| generation > *old)
            {
                last_valid.insert(target.into(), (generation, name.clone()));
            }
            let mut item = json!({"kind":"private_version","name":name,"path":format!(".pillcli/{name}"),"target":target,"generation_ns":generation.to_string(),"sha256":hash(&manifest_bytes),"base_sha256":base_hash,"base_info_sha256":information_sha,"identity":platform::identity(&transaction)?,"operation":"archive_private_version"});
            item["fingerprint"] = json!(hash(item.to_string().as_bytes()));
            candidates.push(item);
        }
        candidates.retain(|item| {
            !last_valid
                .values()
                .any(|(_, name)| item["name"] == name.as_str())
        });
        Ok(candidates)
    }

    pub fn archive_private_version(&self, fingerprint: &str) -> Result<Value, FileError> {
        self.archive_private_version_with(fingerprint, |_, bytes| {
            serde_json::from_slice::<Value>(bytes).is_ok()
        })
    }

    pub fn archive_private_version_with(
        &self,
        fingerprint: &str,
        valid: impl Fn(&str, &[u8]) -> bool,
    ) -> Result<Value, FileError> {
        let _effect = self
            .effects
            .lock()
            .map_err(|_| FileError::Conflict("bloqueo interno".into()))?;
        let private = self.private_directory(true)?;
        let _lock = platform::private_lock_at(&private)?;
        let item = self
            .private_archive_offers_with(&valid)?
            .into_iter()
            .find(|item| item["fingerprint"].as_str() == Some(fingerprint))
            .ok_or_else(|| {
                FileError::Conflict(
                    "la versión privada cambió o es el último respaldo válido".into(),
                )
            })?;
        let name = item["name"]
            .as_str()
            .ok_or_else(|| FileError::Invalid("versión sin nombre".into()))?;
        let source = platform::moving_directory_at(&private, name.as_ref())?;
        if platform::identity(&source)? != item["identity"].as_str().unwrap_or("") {
            return Err(FileError::Conflict("objeto privado reemplazado".into()));
        }
        self.verify_private_version_contents(&source, &item)?;
        let archive = platform::open_directory_at(&private, "archive".as_ref(), true)?;
        let target = platform::open_directory_at(&archive, "private".as_ref(), true)?;
        platform::rename_at(&source, &target, name.as_ref())?;
        let verified = (|| -> Result<String, FileError> {
            let actual = self.verify_private_version_contents(&source, &item)?;
            if folded(&platform::final_path(&source)?)
                != folded(&self.root.join(".pillcli/archive/private").join(name))
            {
                return Err(FileError::Conflict(
                    "nombre de archivo privado no confirmado".into(),
                ));
            }
            Ok(actual)
        })();
        let actual = verified.map_err(|error| FileError::OperationPartial {
            operation: "archive_private".into(),
            path: format!(".pillcli/{name}"),
            destination: Some(format!(".pillcli/archive/private/{name}")),
            reason: format!("versión movida sin confirmación completa: {error}"),
        })?;
        Ok(
            json!({"status":"verified","name":name,"destination":format!(".pillcli/archive/private/{name}"),"sha256":actual}),
        )
    }

    fn verify_private_version_contents(
        &self,
        directory: &File,
        item: &Value,
    ) -> Result<String, FileError> {
        if platform::identity(directory)? != item["identity"].as_str().unwrap_or("") {
            return Err(FileError::Conflict(
                "identidad de versión privada cambió".into(),
            ));
        }
        let (members, truncated) = platform::entries(directory, self.config.max_entries)?;
        let base = item["base_sha256"].as_str();
        let information = item["base_info_sha256"].as_str();
        if truncated
            || members.len() != 1 + usize::from(base.is_some()) + usize::from(information.is_some())
            || !members
                .iter()
                .any(|name| name == OsStr::new("transaction.json"))
            || members.iter().any(|name| {
                name != OsStr::new("transaction.json")
                    && (name != OsStr::new("base") || base.is_none())
                    && (name != OsStr::new("base-info.json") || information.is_none())
            })
        {
            return Err(FileError::Conflict(
                "miembros de versión privada cambiaron".into(),
            ));
        }
        let mut manifest = platform::open_file_at(directory, "transaction.json".as_ref(), false)?;
        let actual = hash(&Self::private_bytes(&mut manifest)?);
        if item["sha256"] != actual {
            return Err(FileError::Conflict(
                "manifiesto de versión privada cambió".into(),
            ));
        }
        if let Some(expected) = information {
            let mut info = platform::open_file_at(directory, "base-info.json".as_ref(), false)?;
            if hash(&Self::private_bytes(&mut info)?) != expected {
                return Err(FileError::Conflict(
                    "sobre inmutable de versión privada cambió".into(),
                ));
            }
            let Some((metadata, sha)) = self.private_recovery_metadata(directory)? else {
                return Err(FileError::Conflict("metadatos privados inválidos".into()));
            };
            let mutable: Value = serde_json::from_slice(&Self::private_bytes(&mut manifest)?)
                .map_err(|e| FileError::Invalid(e.to_string()))?;
            if sha.as_deref() != Some(expected)
                || Self::stable_private_metadata(&metadata)
                    != Self::stable_private_metadata(&mutable)
            {
                return Err(FileError::Conflict(
                    "metadatos privados contradictorios".into(),
                ));
            }
        }
        if let Some(expected) = base {
            let mut file = platform::open_file_at(directory, "base".as_ref(), false)?;
            if hash(&Self::private_bytes(&mut file)?) != expected {
                return Err(FileError::Conflict("base de versión privada cambió".into()));
            }
        }
        Ok(actual)
    }

    fn transaction_locator(relative: &str) -> String {
        hash(
            format!(
                "pill-project-archive-v1\n{}",
                relative.replace('\\', "/").to_lowercase()
            )
            .as_bytes(),
        )
    }

    fn project_transaction_detail(
        &self,
        parent: &File,
        name: &OsStr,
        relative: &str,
    ) -> Result<Option<Value>, FileError> {
        let directory = platform::open_directory_at(parent, name, false)?;
        let (members, truncated) = platform::entries(&directory, self.config.max_entries)?;
        if truncated {
            return Err(FileError::Limit(
                "miembros transaccionales superan files.max_entries".into(),
            ));
        }
        if members.len() != 2
            || !members.iter().any(|m| m == OsStr::new("base"))
            || !members.iter().any(|m| m == OsStr::new("transaction.json"))
        {
            return Ok(None);
        }
        let mut manifest = platform::open_file_at(&directory, "transaction.json".as_ref(), false)?;
        let manifest_bytes = Self::private_bytes(&mut manifest)?;
        let Ok(data) = serde_json::from_slice::<Value>(&manifest_bytes) else {
            return Ok(None);
        };
        if !matches!(data["operation"].as_str(), Some("edit" | "restore"))
            || data["state"] != "verified"
        {
            return Ok(None);
        }
        let Some(path) = data["path"].as_str() else {
            return Ok(None);
        };
        let target = Path::new(path);
        if target.parent().is_none_or(|p| {
            folded(p) != folded(self.root.join(relative).parent().expect("transacción"))
        }) || self.check_path(target).is_err()
        {
            return Ok(None);
        }
        let mut base = platform::open_file_at(&directory, "base".as_ref(), false)?;
        let bytes = self.operation_bytes(&mut base)?;
        let base_sha = hash(&bytes);
        if data["base_sha256"].as_str() != Some(base_sha.as_str())
            || data["intended_sha256"]
                .as_str()
                .is_none_or(|sha| sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Ok(None);
        }
        let mut item = json!({"kind":"project_transaction","name":relative,"path":relative,"target":self.relative(target)?,"backup_path":self.root.join(relative).join("base"),"sha256":hash(&manifest_bytes),"base_sha256":base_sha,"identity":platform::identity(&directory)?,"locator":Self::transaction_locator(relative),"operation":"archive_project_transaction"});
        item["fingerprint"] = json!(hash(item.to_string().as_bytes()));
        Ok(Some(item))
    }

    /// Enumera sin seguir enlaces; no publica una lista truncada como completa.
    pub fn project_archive_offers(&self) -> Result<Value, FileError> {
        let mut pending = vec![".".to_owned()];
        let mut scanned = 0usize;
        let mut omitted = 0usize;
        let mut items = Vec::new();
        while let Some(relative) = pending.pop() {
            let checked = self.resolve(&relative, true)?;
            let directory = checked
                .entry
                .as_ref()
                .ok_or_else(|| FileError::Invalid("directorio sin handle".into()))?;
            let remaining = self
                .config
                .max_entries
                .saturating_sub(scanned + pending.len());
            let (names, truncated) = platform::entries(directory, remaining)?;
            if truncated {
                return Err(FileError::Limit("archivo de respaldos requiere una enumeración completa dentro de files.max_entries".into()));
            }
            for name in names {
                scanned += 1;
                let path = checked.path.join(&name);
                let child = self.relative(&path)?;
                if name.to_string_lossy().starts_with(TRANSACTION_PREFIX) {
                    if let Some(item) = self.project_transaction_detail(directory, &name, &child)? {
                        items.push(item);
                    }
                    continue;
                }
                if self.check_path(&path).is_err() {
                    omitted += 1;
                    continue;
                }
                let entry = match platform::open_any_at(directory, &name) {
                    Ok(entry) => entry,
                    Err(FileError::Protected(_)) => {
                        omitted += 1;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                if platform::is_directory(&entry)? {
                    pending.push(child);
                }
            }
        }
        Ok(json!({"items":items,"scanned":scanned,"omitted":omitted,"truncated":false}))
    }

    fn verify_project_transaction_contents(
        &self,
        directory: &File,
        item: &Value,
    ) -> Result<(), FileError> {
        if platform::identity(directory)? != item["identity"].as_str().unwrap_or("") {
            return Err(FileError::Conflict("transacción reemplazada".into()));
        }
        let (members, truncated) = platform::entries(directory, self.config.max_entries)?;
        if truncated
            || members.len() != 2
            || !members.iter().any(|n| n == OsStr::new("base"))
            || !members.iter().any(|n| n == OsStr::new("transaction.json"))
        {
            return Err(FileError::Conflict(
                "miembros de transacción cambiaron".into(),
            ));
        }
        let mut manifest = platform::open_file_at(directory, "transaction.json".as_ref(), false)?;
        if item["sha256"] != hash(&Self::private_bytes(&mut manifest)?) {
            return Err(FileError::Conflict(
                "manifiesto transaccional cambió".into(),
            ));
        }
        let mut base = platform::open_file_at(directory, "base".as_ref(), false)?;
        if item["base_sha256"] != hash(&self.operation_bytes(&mut base)?) {
            return Err(FileError::Conflict("respaldo transaccional cambió".into()));
        }
        Ok(())
    }

    pub fn archive_project_transaction(&self, fingerprint: &str) -> Result<Value, FileError> {
        let _effect = self
            .effects
            .lock()
            .map_err(|_| FileError::Conflict("bloqueo interno".into()))?;
        let offers = self.project_archive_offers()?;
        let item = offers["items"]
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item["fingerprint"].as_str() == Some(fingerprint))
            })
            .ok_or_else(|| {
                FileError::Conflict("la transacción cambió; vuelve a mostrar archivo".into())
            })?;
        let relative = item["path"]
            .as_str()
            .ok_or_else(|| FileError::Invalid("transacción sin ruta".into()))?;
        let relative_path = Path::new(relative);
        let parent_path = relative_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = self.resolve(
            parent_path
                .to_str()
                .ok_or_else(|| FileError::Invalid("ruta de respaldo no Unicode".into()))?,
            true,
        )?;
        let source = platform::moving_directory_at(
            parent.entry.as_ref().expect("directorio"),
            relative_path.file_name().expect("nombre"),
        )?;
        self.verify_project_transaction_contents(&source, item)?;
        let private = self.private_directory(true)?;
        let archive = platform::open_directory_at(&private, "archive".as_ref(), true)?;
        let transactions = platform::open_directory_at(&archive, "transactions".as_ref(), true)?;
        let locator = item["locator"]
            .as_str()
            .ok_or_else(|| FileError::Invalid("localizador ausente".into()))?;
        platform::rename_at(&source, &transactions, locator.as_ref())?;
        let check = (|| -> Result<(), FileError> {
            self.verify_project_transaction_contents(&source, item)?;
            if folded(&platform::final_path(&source)?)
                != folded(
                    &self
                        .root
                        .join(".pillcli/archive/transactions")
                        .join(locator),
                )
            {
                return Err(FileError::Conflict(
                    "archivo transaccional sin nombre confirmado".into(),
                ));
            }
            Ok(())
        })();
        check.map_err(|error| FileError::OperationPartial {
            operation: "archive_transaction".into(),
            path: relative.into(),
            destination: Some(format!(".pillcli/archive/transactions/{locator}")),
            reason: error.to_string(),
        })?;
        Ok(
            json!({"status":"verified","path":relative,"destination":format!(".pillcli/archive/transactions/{locator}"),"locator":locator,"sha256":item["sha256"],"base_sha256":item["base_sha256"],"backup_path":item["backup_path"]}),
        )
    }

    fn transaction(&self, parent_handle: &File, parent: &Path) -> Result<Transaction, FileError> {
        let mut count = 0;
        let (entries, truncated) = platform::entries(parent_handle, self.config.max_entries)?;
        if truncated {
            return Err(FileError::Limit(
                "directorio demasiado grande para verificar cuota transaccional".into(),
            ));
        }
        for entry in entries {
            if entry
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with(TRANSACTION_PREFIX)
            {
                count += 1;
                if count >= self.config.max_transactions {
                    return Err(FileError::Limit(
                        "máximo de backups transaccionales; revisión manual necesaria".into(),
                    ));
                }
            }
        }
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| FileError::Invalid(error.to_string()))?
            .as_nanos();
        let sequence = TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            "{TRANSACTION_PREFIX}{}-{time}-{sequence}",
            std::process::id()
        );
        let pin = platform::create_directory_at(parent_handle, name.as_ref())?;
        Ok(Transaction {
            path: parent.join(name),
            pin,
        })
    }

    fn read_bytes(&self, file: &mut File) -> Result<Vec<u8>, FileError> {
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.take(self.config.max_file_bytes as u64 + 1)
            .read_to_end(&mut bytes)?;
        self.validate_text(&bytes)?;
        Ok(bytes)
    }

    fn validate_text(&self, bytes: &[u8]) -> Result<(), FileError> {
        if bytes.len() > self.config.max_file_bytes {
            return Err(FileError::Limit(
                "archivo supera max_file_bytes; no se recorta para editar".into(),
            ));
        }
        decode(bytes)?;
        Ok(())
    }

    fn relative(&self, path: &Path) -> Result<String, FileError> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| FileError::Protected(path.display().to_string()))?;
        if relative.as_os_str().is_empty() {
            return Ok(".".into());
        }
        Ok(relative.to_string_lossy().replace('\\', "/"))
    }

    fn check_handle(&self, file: &File) -> Result<(), FileError> {
        platform::check_regular(file)?;
        self.check_path(&platform::final_path(file)?)
    }

    fn check_path(&self, path: &Path) -> Result<(), FileError> {
        let actual = folded(path);
        let root = folded(&self.root);
        if actual != root && !actual.starts_with(&(root + "\\")) {
            return Err(FileError::Protected(path.display().to_string()));
        }
        if self.protected_paths.iter().any(|protected| {
            let protected = folded(protected);
            actual == protected || actual.starts_with(&(protected + "\\"))
        }) {
            return Err(FileError::Protected(path.display().to_string()));
        }
        for part in path.components().filter_map(|part| match part {
            Component::Normal(name) => Some(name.to_string_lossy()),
            _ => None,
        }) {
            let lower = part.to_lowercase();
            if CONTROL_COMPONENTS.contains(&lower.as_str())
                || lower.starts_with(TRANSACTION_PREFIX)
                || self
                    .config
                    .excluded_components
                    .iter()
                    .any(|excluded| excluded.eq_ignore_ascii_case(&part))
            {
                return Err(FileError::Protected(path.display().to_string()));
            }
        }
        Ok(())
    }

    fn resolve(&self, relative: &str, directory_allowed: bool) -> Result<Resolved, FileError> {
        if relative.is_empty() || relative.chars().any(char::is_control) {
            return Err(FileError::Invalid("ruta vacía o con controles".into()));
        }
        let mut path = self.root.clone();
        let mut names = Vec::new();
        for component in Path::new(relative).components() {
            match component {
                Component::CurDir if relative == "." => {}
                Component::Normal(name) => {
                    let text = name.to_string_lossy();
                    if text.contains([':', '*', '?', '<', '>', '|', '"'])
                        || text.ends_with(['.', ' '])
                        || reserved_device(&text)
                    {
                        return Err(FileError::Invalid(format!(
                            "nombre Windows inválido: {text}"
                        )));
                    }
                    names.push(name.to_os_string());
                }
                _ => return Err(FileError::Protected(relative.into())),
            }
        }
        let mut pins = Vec::new();
        let mut parent = self._root_pins.last().expect("raíz fijada").try_clone()?;
        if names.is_empty() {
            self.check_path(&path)?;
            if !directory_allowed {
                return Err(FileError::Invalid("se requiere archivo".into()));
            }
            return Ok(Resolved {
                path,
                parent: parent.try_clone()?,
                leaf: OsString::new(),
                entry: Some(parent),
                _pins: pins,
            });
        }
        for (index, name) in names.iter().enumerate() {
            path.push(name);
            self.check_path(&path)?;
            if index + 1 < names.len() {
                let pin = platform::open_directory_at(&parent, name, false)?;
                path = platform::final_path(&pin)?;
                self.check_path(&path)?;
                parent = pin.try_clone()?;
                pins.push(pin);
            }
        }
        self.check_path(&path)?;
        let leaf = names.last().expect("nombre final").clone();
        let entry = if directory_allowed {
            let file = platform::open_any_at(&parent, &leaf)?;
            if !platform::is_directory(&file)? {
                self.check_handle(&file)?;
            }
            path = platform::final_path(&file)?;
            self.check_path(&path)?;
            Some(file)
        } else {
            None
        };
        Ok(Resolved {
            path,
            parent,
            leaf,
            entry,
            _pins: pins,
        })
    }
}

struct Resolved {
    path: PathBuf,
    parent: File,
    leaf: OsString,
    entry: Option<File>,
    _pins: Vec<File>,
}

struct OperationSnapshot {
    resolved: Resolved,
    file: File,
    kind: &'static str,
    sha256: String,
    bytes: usize,
    identity: String,
    utf8_text: bool,
}

struct Transaction {
    path: PathBuf,
    pin: File,
}

fn write_manifest(file: &mut File, data: &Value) -> Result<(), FileError> {
    let bytes = data.to_string();
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    file.write_all(bytes.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn publish_staged(
    original: &File,
    staged: &File,
    destination_parent: &File,
    destination: &std::ffi::OsStr,
    backup: &Path,
    path: &str,
) -> Result<(), FileError> {
    if let Err(error) = platform::rename_at(staged, destination_parent, destination) {
        if platform::rename_at(original, destination_parent, destination).is_ok() {
            return Err(FileError::Conflict(format!(
                "publicación falló; base restaurada sin sobrescribir: {error}"
            )));
        }
        return Err(FileError::Partial {
            path: path.into(),
            backup_path: backup.display().to_string(),
            reason: format!("publicación y restauración rechazadas sin sobrescribir: {error}"),
        });
    }
    Ok(())
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn decode(bytes: &[u8]) -> Result<(&str, bool), FileError> {
    let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
    let body = if bom { &bytes[3..] } else { bytes };
    let text = std::str::from_utf8(body)
        .map_err(|_| FileError::Invalid("solo UTF-8; no se convirtió el archivo".into()))?;
    if text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\r' | '\n' | '\t'))
    {
        return Err(FileError::Invalid(
            "contenido binario o controles no admitidos".into(),
        ));
    }
    Ok((text, bom))
}

fn line_endings(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count();
    let cr = text.matches('\r').count();
    if lf == 0 && cr == 0 {
        "none"
    } else if crlf == lf && crlf == cr {
        "crlf"
    } else if cr == 0 {
        "lf"
    } else {
        "mixed"
    }
}

fn folded(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn reserved_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

#[cfg(windows)]
mod platform;

#[cfg(all(test, windows))]
mod tests;

#[cfg(not(windows))]
compile_error!("El contrato de archivos E3 requiere Windows y sus handles de compartición.");
