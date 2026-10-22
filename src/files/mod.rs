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
const PRIVATE_NAMES: &[&str] = &["console.json", "model.json"];
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

    /// Lee `console.json` o `model.json` bajo el pin de `.pillcli`. No crea el archivo.
    pub fn read_private(&self, name: &str) -> Result<Option<Vec<u8>>, FileError> {
        self.private_name(name)?;
        let directory = match self.private_directory(false) {
            Ok(directory) => directory,
            Err(FileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let mut file = match platform::open_file_at(&directory, OsStr::new(name), false) {
            Ok(file) => file,
            Err(FileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let length = file.metadata()?.len();
        if length > JOURNAL_TAIL_BYTES {
            return Err(FileError::Limit("archivo privado demasiado grande".into()));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(Some(bytes))
    }

    /// Sustituye `console.json` o `model.json` bajo el pin de `.pillcli`.
    pub fn write_private(&self, name: &str, bytes: &[u8]) -> Result<(), FileError> {
        self.private_name(name)?;
        if bytes.len() as u64 > JOURNAL_TAIL_BYTES {
            return Err(FileError::Limit("archivo privado demasiado grande".into()));
        }
        let directory = self.private_directory(true)?;
        let mut file = platform::overwrite_file_at(&directory, OsStr::new(name))?;
        file.set_len(0)?;
        file.write_all(bytes)?;
        file.sync_all()?;
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
        let _effect = self
            .effects
            .lock()
            .map_err(|_| FileError::Conflict("bloqueo interno".into()))?;
        if expected_sha256.len() != 64
            || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(FileError::Invalid(
                "sha256 debe tener 64 dígitos hexadecimales".into(),
            ));
        }
        if old.is_empty() {
            return Err(FileError::Invalid("old no puede estar vacío".into()));
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
        if text.match_indices(old).count() != 1 {
            return Err(FileError::Conflict(
                "old debe coincidir exactamente una vez".into(),
            ));
        }
        let after_text = text.replacen(old, new, 1);
        let mut after = Vec::new();
        if bom {
            after.extend_from_slice(&[0xef, 0xbb, 0xbf]);
        }
        after.extend_from_slice(after_text.as_bytes());
        self.validate_text(&after)?;
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
        let mut manifest_data = json!({"operation":"edit","path":resolved.path,
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
            "diff":{"before":text,"after":after_text},"utf8_bom":bom}),
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
