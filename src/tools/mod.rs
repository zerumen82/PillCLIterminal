pub mod commands;
pub mod mcp;
mod process_job;
mod read_pool;
pub mod recovery;

use crate::{config::Config, files::Files, providers::ollama::ToolCall};
use commands::CommandRunner;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Coordinator,
    Explorer,
    Implementer,
    Tester,
    Reviewer,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Self::Coordinator => "coordinador",
            Self::Explorer => "explorador",
            Self::Implementer => "implementador",
            Self::Tester => "tester",
            Self::Reviewer => "reviewer",
        }
    }
    fn permits(self, name: &str) -> bool {
        match name {
            "read_file" | "list_files" | "search_files" | "inspect_path" => true,
            "create_file" | "edit_file" | "edit_file_many" | "move_file" | "delete_file" => {
                self == Self::Implementer
            }
            "run_command" => self == Self::Tester || self == Self::Implementer,
            name if name.starts_with("mcp__") => self == Self::Tester || self == Self::Implementer,
            _ => false,
        }
    }
}

pub fn schemas(role: Role, commands_enabled: bool) -> Vec<Value> {
    let definitions = [
        (
            "list_files",
            "Lista directorio; . es raíz.",
            json!({"path":{"type":"string"}}),
            vec!["path"],
        ),
        (
            "read_file",
            "Lee texto y SHA256.",
            json!({"path":{"type":"string"}}),
            vec!["path"],
        ),
        (
            "search_files",
            "Busca texto literal.",
            json!({"path":{"type":"string"},"needle":{"type":"string"}}),
            vec!["path", "needle"],
        ),
        (
            "create_file",
            "Crea sin sobrescribir; padre existente.",
            json!({"path":{"type":"string"},"content":{"type":"string"}}),
            vec!["path", "content"],
        ),
        (
            "edit_file",
            "Reemplaza coincidencia única tras leer; respalda.",
            json!({"path":{"type":"string"},"expected_sha256":{"type":"string"},"old":{"type":"string"},"new":{"type":"string"}}),
            vec!["path", "expected_sha256", "old", "new"],
        ),
        (
            "edit_file_many",
            "Reemplazos únicos ordenados, una base SHA; publica todos o ninguno.",
            json!({"path":{"type":"string"},"expected_sha256":{"type":"string"},"edits":{"type":"array","minItems":1,"items":{"type":"object","properties":{"old":{"type":"string"},"new":{"type":"string"}},"required":["old","new"],"additionalProperties":false}}}),
            vec!["path", "expected_sha256", "edits"],
        ),
        (
            "inspect_path",
            "Versión para mover/borrar; inspecciona ausencia.",
            json!({"path":{"type":"string"}}),
            vec!["path"],
        ),
        (
            "move_file",
            "Mueve sin sobrescribir; SHA observado y concesión en /permissions.",
            json!({"path":{"type":"string"},"destination":{"type":"string"},"expected_sha256":{"type":"string"}}),
            vec!["path", "destination", "expected_sha256"],
        ),
        (
            "delete_file",
            "Borra archivo/carpeta vacía; SHA observado y concesión en /permissions.",
            json!({"path":{"type":"string"},"expected_sha256":{"type":"string"}}),
            vec!["path", "expected_sha256"],
        ),
        (
            "run_command",
            "Ejecuta id concedido por usuario.",
            json!({"id":{"type":"string"}}),
            vec!["id"],
        ),
    ];
    definitions.into_iter().filter(|(name,_,_,_)| role.permits(name) && (*name!="run_command" || commands_enabled)).map(|(name,description,properties,required)| json!({"type":"function","function":{"name":name,"description":description,"parameters":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}})).collect()
}

pub struct Toolset {
    pub files: Arc<Files>,
    pub commands: Option<CommandRunner>,
    trust: CommandRunner,
    mcp: mcp::Catalog,
    gpu: crate::gpu::GpuConfig,
    ollama_url: String,
    _pins: Vec<File>,
    max_journal_bytes: usize,
    max_sessions: usize,
    max_record_bytes: usize,
    versions: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    observations: Arc<std::sync::Mutex<std::collections::BTreeMap<String, String>>>,
    text_observations: Arc<std::sync::Mutex<std::collections::BTreeMap<String, String>>>,
    tested: std::sync::Mutex<std::collections::BTreeMap<String, TestEvidence>>,
    uncertain_effects: AtomicBool,
    file_offers: std::sync::Mutex<std::collections::BTreeMap<String, FileOffer>>,
    recovery_text: String,
    read_pool: read_pool::ReadPool,
    changes: std::sync::Mutex<Vec<Value>>,
    max_detail_entries: usize,
}

struct FileOffer {
    operation: String,
    path: String,
    destination: Option<String>,
    sha256: String,
    fingerprint: String,
    granted: bool,
}

struct TestEvidence {
    fingerprint: String,
    versions: std::collections::BTreeMap<String, String>,
}

impl Toolset {
    pub fn new(
        project: &Path,
        config: &Config,
        source: Option<&Path>,
        trust: &Path,
    ) -> Result<Self, String> {
        let installation = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut protected: Vec<PathBuf> = [
            "src",
            "config",
            "bin",
            "Cargo.toml",
            "Cargo.lock",
            "scripts",
        ]
        .iter()
        .map(|part| installation.join(part))
        .filter(|p| p.exists())
        .collect();
        if let Some(path) = source {
            protected.push(path.to_owned());
        }
        let files = Files::new(project, config.files.clone())
            .and_then(|files| files.with_protected_paths(protected))
            .map_err(|e| e.to_string())?;
        let (_, pins) = files.session_directory().map_err(|e| e.to_string())?;
        let trust_runner = CommandRunner::new(project, config.commands.clone(), trust)
            .map_err(|e| e.to_string())?;
        let commands = trust_runner.has_definitions().then(|| trust_runner.clone());
        let mcp = mcp::Catalog::new(project, config.mcp.clone())?;
        let recovered = recovery::reconcile(
            &files,
            config.agents.max_sessions,
            config.agents.max_journal_bytes,
            config.agents.max_record_bytes,
        );
        let files = Arc::new(files);
        let observations = Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new()));
        let text_observations = Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new()));
        let read_pool = read_pool::ReadPool::new(
            config.agents.max_parallel_reads,
            read_pool::Context {
                files: files.clone(),
                observations: observations.clone(),
                text_observations: text_observations.clone(),
            },
        )?;
        Ok(Self {
            read_pool,
            changes: std::sync::Mutex::new(Vec::new()),
            max_detail_entries: config.ui.max_detail_entries,
            files,
            commands,
            trust: trust_runner,
            mcp,
            gpu: config.gpu.clone(),
            ollama_url: config.ollama.url.clone(),
            _pins: pins,
            max_journal_bytes: config.agents.max_journal_bytes,
            max_sessions: config.agents.max_sessions,
            max_record_bytes: config.agents.max_record_bytes,
            versions: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            observations,
            text_observations,
            tested: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            uncertain_effects: AtomicBool::new(recovered.needs_review),
            recovery_text: recovered.summary(),
            file_offers: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        })
    }

    pub fn journal_byte_limit(&self) -> usize {
        self.max_journal_bytes
    }

    pub fn recovery_view(&self) -> Result<Value, String> {
        let report = recovery::reconcile(
            &self.files,
            self.max_sessions,
            self.max_journal_bytes,
            self.max_record_bytes,
        );
        self.uncertain_effects
            .store(report.needs_review, Ordering::SeqCst);
        let mut value = serde_json::to_value(&report).map_err(|e| e.to_string())?;
        value["summary"] = json!(report.summary());
        Ok(value)
    }
    pub fn acknowledge_recovery(&self, fingerprint: &str) -> Result<Value, String> {
        let report = recovery::acknowledge(
            &self.files,
            fingerprint,
            self.max_sessions,
            self.max_journal_bytes,
            self.max_record_bytes,
        )?;
        self.uncertain_effects
            .store(report.needs_review, Ordering::SeqCst);
        let mut value = serde_json::to_value(&report).map_err(|e| e.to_string())?;
        value["summary"] = json!(report.summary());
        Ok(value)
    }
    pub fn changes(&self) -> Vec<Value> {
        self.changes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn retain_change(&self, name: &str, result: &Value) {
        if result["status"] != "verified" {
            return;
        }
        let mut change = result.clone();
        change["name"] = json!(name);
        let mut changes = self.changes.lock().unwrap_or_else(|e| e.into_inner());
        changes.push(change);
        if changes.len() > self.max_detail_entries {
            changes.remove(0);
        }
    }
    pub fn undo(
        &self,
        path: &str,
        expected: &str,
        backup: &str,
        backup_sha: &str,
    ) -> Result<Value, String> {
        if self.uncertain_effects.load(Ordering::SeqCst) {
            return Err("efectos parciales sin revisar; inspecciona /recover".into());
        }
        if !self.changes().iter().any(|c| {
            c["path"] == path
                && c["sha256"] == expected
                && c["backup_path"] == backup
                && c["backup_sha256"] == backup_sha
        }) {
            return Err("respaldo no propuesto por cambios observados de esta sesión".into());
        }
        let call = ToolCall {
            function: crate::providers::ollama::ToolFunction {
                name: "restore_backup".into(),
                arguments: json!({"path":path,"expected_sha256":expected,"backup_path":backup,"backup_sha256":backup_sha}),
            },
        };
        let mut journal = self.journal()?;
        let id = journal.requested(Role::Implementer, &call, Some(backup_sha))?;
        let result = self
            .files
            .restore_backup(path, expected, backup, backup_sha)
            .unwrap_or_else(|e| e.data());
        if matches!(
            result["status"].as_str(),
            Some("partial" | "unknown" | "applied")
        ) {
            self.block_effects();
        }
        if let Err(error) = journal.observed(Role::Implementer, &call, &result, &id) {
            self.block_effects();
            return Err(error);
        }
        if result["status"] == "verified" {
            self.reset_versions();
            if let Some(sha) = result["sha256"].as_str() {
                self.versions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(path.into(), sha.into());
            }
            self.retain_change("restore_backup", &result);
        }
        Ok(result)
    }
    pub fn archive_offers(&self) -> Result<Value, String> {
        recovery::archive_offers(
            &self.files,
            self.max_sessions,
            self.max_journal_bytes,
            self.max_record_bytes,
        )
    }
    pub fn confirm_archive(&self, fingerprint: &str) -> Result<Value, String> {
        recovery::confirm_archive(
            &self.files,
            fingerprint,
            self.max_sessions,
            self.max_journal_bytes,
            self.max_record_bytes,
        )
    }
    pub fn retention_status(&self) -> Result<Value, String> {
        self.files.retention_status().map_err(|e| e.to_string())
    }

    pub fn recovery_report(&self) -> &str {
        &self.recovery_text
    }

    pub fn block_effects(&self) {
        self.uncertain_effects.store(true, Ordering::SeqCst);
    }

    pub fn reset_versions(&self) {
        self.text_observations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.versions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.observations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.tested
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
    pub fn changed_paths(&self) -> Vec<String> {
        self.versions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }
    pub fn verify_versions(&self) -> Result<(), String> {
        let versions = self
            .versions
            .lock()
            .map_err(|_| "registro de versiones bloqueado")?
            .clone();
        self.verify_snapshot(&versions)
    }

    fn verify_snapshot(
        &self,
        versions: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), String> {
        for (path, sha) in versions {
            if sha.is_empty() {
                match self.files.inspect_operation(path) {
                    Err(crate::files::FileError::Io(e))
                        if e.kind() == std::io::ErrorKind::NotFound =>
                    {
                        continue;
                    }
                    _ => {
                        return Err(format!(
                            "{path} dejó de estar ausente; verificación invalidada"
                        ));
                    }
                }
            }
            let read = self
                .files
                .inspect_operation(path)
                .map_err(|e| e.to_string())?;
            if read["sha256"].as_str() != Some(sha.as_str()) {
                return Err(format!(
                    "la versión de {path} cambió tras aplicar/probar; verificación invalidada"
                ));
            }
        }
        Ok(())
    }

    pub fn verify_task_evidence(&self) -> Result<(), String> {
        if self.uncertain_effects.load(Ordering::SeqCst) {
            return Err("hay efectos parciales sin reconciliar; revisa diario y backups antes de otra sesión".into());
        }
        self.verify_versions()?;
        let tested = self
            .tested
            .lock()
            .map_err(|_| "evidencia de pruebas bloqueada")?;
        let runner = self.commands.as_ref().ok_or("sin ejecutor de pruebas")?;
        for (id, evidence) in tested.iter() {
            runner
                .verify_fingerprint(id, &evidence.fingerprint)
                .map_err(|e| e.to_string())?;
            self.verify_snapshot(&evidence.versions)?;
        }
        Ok(())
    }

    fn snapshot(&self) -> Result<std::collections::BTreeMap<String, String>, String> {
        let mut snapshot = self
            .observations
            .lock()
            .map_err(|_| "registro de lecturas bloqueado")?
            .clone();
        snapshot.extend(
            self.versions
                .lock()
                .map_err(|_| "registro de versiones bloqueado")?
                .clone(),
        );
        Ok(snapshot)
    }

    async fn run_command(&self, id: &str, cancel: Arc<AtomicBool>) -> Result<Value, String> {
        let runner = self
            .commands
            .as_ref()
            .ok_or("no hay comandos registrados ni concedidos")?;
        let snapshot = self.snapshot()?;
        self.verify_snapshot(&snapshot)?;
        let result = runner
            .execute(id, cancel)
            .await
            .map_err(|e| e.to_string())?;
        let changed = self.verify_snapshot(&snapshot).err();
        let verified = result.exit_code == Some(0)
            && !result.timed_out
            && !result.cancelled
            && changed.is_none();
        if verified {
            self.tested
                .lock()
                .map_err(|_| "evidencia de pruebas bloqueada")?
                .insert(
                    id.into(),
                    TestEvidence {
                        fingerprint: result.fingerprint.clone(),
                        versions: snapshot,
                    },
                );
        }
        Ok(
            json!({"status":if result.timed_out || result.cancelled {"unknown"} else if verified {"verified"} else {"failed"},"command":result,"version_error":changed}),
        )
    }

    pub fn journal(&self) -> Result<Journal, String> {
        if self.files.session_count().map_err(|e| e.to_string())? >= self.max_sessions {
            return Err(
                "límite de sesiones locales alcanzado; revisa los diarios antes de continuar"
                    .into(),
            );
        }
        let id = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
        );
        let (path, pins, file) = self
            .files
            .open_session_journal(&format!("{id}.jsonl"))
            .map_err(|e| e.to_string())?;
        Ok(Journal {
            file,
            path,
            id,
            bytes: 0,
            max: self.max_journal_bytes,
            sequence: 0,
            record_max: self.max_record_bytes,
            _pins: pins,
            reserve: 0,
        })
    }

    pub fn commands_enabled(&self) -> bool {
        self.commands.is_some()
    }

    pub fn nvidia_path(&self) -> Option<&Path> {
        self.gpu.nvidia_smi.as_deref()
    }

    pub fn exclusive_granted(&self) -> Result<bool, String> {
        if !self.gpu.exclusive {
            return Ok(false);
        }
        let expected = crate::gpu::exclusive_fingerprint(&self.ollama_url);
        let grants = self
            .trust
            .load_grants()
            .map_err(|error| error.to_string())?;
        Ok(grants.exclusive.get(self.trust.project_root()) == Some(&expected))
    }

    pub fn offers(&self) -> Result<Vec<mcp::Offer>, String> {
        let mut offers = Vec::new();
        let mut pending = self
            .file_offers
            .lock()
            .map_err(|_| "propuestas bloqueadas")?;
        let mut stale = Vec::new();
        for (id, item) in pending.iter() {
            let detail = self.files.operation_offer(
                &item.operation,
                &item.path,
                item.destination.as_deref(),
                &item.sha256,
            );
            let Ok(mut detail) = detail else {
                stale.push(id.clone());
                continue;
            };
            if detail["fingerprint"].as_str() != Some(&item.fingerprint) {
                stale.push(id.clone());
                continue;
            }
            detail["granted"] = json!(item.granted);
            offers.push(mcp::Offer {
                id: id.clone(),
                fingerprint: item.fingerprint.clone(),
                detail: detail.to_string(),
            });
        }
        for id in stale {
            pending.remove(&id);
        }
        drop(pending);
        if let Some(runner) = &self.commands {
            for proposal in runner.proposals().map_err(|error| error.to_string())? {
                offers.push(mcp::Offer {
                    id: format!("command:{}", proposal.spec.id),
                    fingerprint: proposal.fingerprint.clone(),
                    detail: serde_json::to_string(&proposal).map_err(|error| error.to_string())?,
                });
            }
        }
        offers.extend(self.mcp.offers(&self.trust)?);
        let exclusive = self.exclusive_granted()?;
        for (index, root) in self.files.proposed_roots().iter().enumerate() {
            offers.push(mcp::Offer {
                id: format!("root:{index}"),
                fingerprint: "rechazada".into(),
                detail: format!("raíz extra propuesta y no aplicada: {}", root.display()),
            });
        }
        if self.gpu.exclusive {
            offers.push(mcp::Offer {
                id: "exclusive".into(),
                fingerprint: crate::gpu::exclusive_fingerprint(&self.ollama_url),
                detail: format!(
                    "exclusividad sobre {}; concedida={exclusive}. Solo descargará modelos que esta sesión seleccione.",
                    self.ollama_url
                ),
            });
        }
        Ok(offers)
    }

    pub fn grant_offer(&self, id: &str, fingerprint: &str) -> Result<(), String> {
        if id.starts_with("file:") {
            let mut offers = self
                .file_offers
                .lock()
                .map_err(|_| "propuestas bloqueadas")?;
            let item = offers.get_mut(id).ok_or("operación no propuesta")?;
            let current = self
                .files
                .operation_offer(
                    &item.operation,
                    &item.path,
                    item.destination.as_deref(),
                    &item.sha256,
                )
                .map_err(|e| e.to_string())?;
            if fingerprint != item.fingerprint
                || current["fingerprint"].as_str() != Some(fingerprint)
            {
                return Err("la operación cambió; vuelve a inspeccionar y proponer".into());
            }
            item.granted = true;
            return Ok(());
        }
        if let Some(command) = id.strip_prefix("command:") {
            let runner = self
                .commands
                .as_ref()
                .ok_or("no hay comandos registrados")?;
            return runner
                .grant(command, fingerprint)
                .map_err(|error| error.to_string());
        }
        if let Some(server) = id.strip_prefix("mcp:") {
            return self.mcp.grant(&self.trust, server, fingerprint);
        }
        if id.starts_with("root:") {
            return Err("una raíz extra no amplía el alcance".into());
        }
        if id == "exclusive" {
            if !self.gpu.exclusive {
                return Err("gpu.exclusive está en falso; no hay exclusividad que conceder".into());
            }
            let expected = crate::gpu::exclusive_fingerprint(&self.ollama_url);
            if fingerprint != expected {
                return Err("la propuesta de exclusividad cambió; vuelve a mostrarla".into());
            }
            let mut grants = self
                .trust
                .load_grants()
                .map_err(|error| error.to_string())?;
            grants
                .exclusive
                .insert(self.trust.project_root().to_path_buf(), expected);
            return self
                .trust
                .save_grants(&grants)
                .map_err(|error| error.to_string());
        }
        Err(format!("propuesta desconocida: {id}"))
    }

    pub fn revoke_offer(&self, id: &str) -> Result<(), String> {
        if id.starts_with("file:") {
            self.file_offers
                .lock()
                .map_err(|_| "propuestas bloqueadas")?
                .remove(id);
            return Ok(());
        }
        if let Some(command) = id.strip_prefix("command:") {
            let runner = self
                .commands
                .as_ref()
                .ok_or("no hay comandos registrados")?;
            return runner.revoke(command).map_err(|error| error.to_string());
        }
        if let Some(server) = id.strip_prefix("mcp:") {
            return self.mcp.revoke(&self.trust, server);
        }
        if id.starts_with("root:") {
            return Err("una raíz extra no tiene concesión que retirar".into());
        }
        if id == "exclusive" {
            let mut grants = self
                .trust
                .load_grants()
                .map_err(|error| error.to_string())?;
            grants.exclusive.remove(self.trust.project_root());
            return self
                .trust
                .save_grants(&grants)
                .map_err(|error| error.to_string());
        }
        Err(format!("propuesta desconocida: {id}"))
    }

    pub async fn mcp_schemas(&self) -> (Vec<Value>, Vec<String>) {
        self.mcp.schemas(&self.trust).await
    }

    pub fn is_native_read(call: &ToolCall) -> bool {
        matches!(
            call.function.name.as_str(),
            "read_file" | "list_files" | "search_files" | "inspect_path"
        )
    }

    pub fn execute_reads(
        &self,
        role: Role,
        calls: &[ToolCall],
        cancel: Arc<AtomicBool>,
    ) -> Vec<Value> {
        self.execute_reads_traced(role, calls, cancel)
            .into_iter()
            .map(|(value, _)| value)
            .collect()
    }

    pub fn execute_reads_traced(
        &self,
        role: Role,
        calls: &[ToolCall],
        cancel: Arc<AtomicBool>,
    ) -> Vec<(Value, Value)> {
        self.read_pool.execute(role, calls, cancel)
    }

    pub fn intended_sha256(&self, call: &ToolCall) -> Option<String> {
        let args = &call.function.arguments;
        if call.function.name == "create_file" {
            return args["content"]
                .as_str()
                .map(|s| format!("{:x}", Sha256::digest(s.as_bytes())));
        }
        if !matches!(call.function.name.as_str(), "edit_file" | "edit_file_many") {
            return None;
        }
        let read = self.files.read(args["path"].as_str()?).ok()?;
        if read["sha256"].as_str()? != args["expected_sha256"].as_str()? {
            return None;
        }
        let text = read["content"].as_str()?;
        let patches = if call.function.name == "edit_file" {
            vec![json!({"old":args["old"],"new":args["new"]})]
        } else {
            args["edits"].as_array()?.clone()
        };
        let edits = patches
            .into_iter()
            .map(serde_json::from_value::<crate::files::ExactEdit>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let mut before = Vec::new();
        if read["utf8_bom"] == true {
            before.extend_from_slice(&[0xef, 0xbb, 0xbf]);
        }
        before.extend_from_slice(text.as_bytes());
        let bytes = crate::files::transform_exact(&before, &edits).ok()?;
        Some(format!("{:x}", Sha256::digest(&bytes)))
    }

    pub async fn execute(&self, role: Role, call: &ToolCall, cancel: Arc<AtomicBool>) -> Value {
        if cancel.load(Ordering::SeqCst) {
            return json!({"status":"cancelled","error":"cancelado antes de iniciar herramienta"});
        }
        let name = call.function.name.as_str();
        let effect = matches!(
            name,
            "create_file"
                | "edit_file"
                | "edit_file_many"
                | "move_file"
                | "delete_file"
                | "run_command"
        ) || name.starts_with("mcp__");
        if effect && self.uncertain_effects.load(Ordering::SeqCst) {
            return json!({"status":"failed","error":"efectos parciales pendientes; revisa diario y backups antes de reiniciar la sesión"});
        }
        if !role.permits(name) {
            return json!({"status":"failed","error":"herramienta desconocida o no concedida a este rol"});
        }
        if Self::is_native_read(call) {
            return self
                .read_pool
                .execute(role, std::slice::from_ref(call), cancel)
                .remove(0)
                .0;
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct PathArgs {
            path: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct SearchArgs {
            path: String,
            needle: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct CreateArgs {
            path: String,
            content: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct EditArgs {
            path: String,
            expected_sha256: String,
            old: String,
            new: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct CommandArgs {
            id: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ManyArgs {
            path: String,
            expected_sha256: String,
            edits: Vec<crate::files::ExactEdit>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct OperationArgs {
            path: String,
            expected_sha256: String,
            destination: Option<String>,
        }
        let args = call.function.arguments.clone();
        let result:Result<Value,String> = match name {
            "edit_file_many"=>serde_json::from_value::<ManyArgs>(args).map_err(|e|e.to_string()).and_then(|a| {
                if self.text_observations.lock().map_err(|_|"lecturas bloqueadas")?.get(&a.path).is_none_or(|sha|!sha.eq_ignore_ascii_case(&a.expected_sha256)) {return Err("debes leer antes la ruta con read_file y usar su SHA256".into());}
                self.files.edit_many(&a.path,&a.expected_sha256,&a.edits).map_err(|e|e.data().to_string())
            }),
            "inspect_path" => serde_json::from_value::<PathArgs>(args).map_err(|e|e.to_string()).and_then(|a| {
                match self.files.inspect_operation(&a.path) {
                    Ok(mut value)=>{value["status"]=json!("verified");value["path"]=json!(a.path);Ok(value)},
                    Err(crate::files::FileError::Io(e)) if e.kind()==std::io::ErrorKind::NotFound=>Ok(json!({"status":"verified","path":a.path,"absent":true})),
                    Err(e)=>Err(e.data().to_string()),
                }
            }),
            "move_file"|"delete_file"=>serde_json::from_value::<OperationArgs>(args).map_err(|e|e.to_string()).and_then(|a| {
                if self.observations.lock().map_err(|_|"lecturas bloqueadas")?.get(&a.path).is_none_or(|sha|!sha.eq_ignore_ascii_case(&a.expected_sha256)) {return Err("inspecciona primero la ruta con read_file o inspect_path y usa su SHA256".into());}
                let operation = if name=="move_file" {"move"} else {"delete"};
                if (operation=="move") != a.destination.is_some() {return Err("destination es obligatorio solo para mover".into());}
                let detail=self.files.operation_offer(operation,&a.path,a.destination.as_deref(),&a.expected_sha256).map_err(|e|e.data().to_string())?;
                let fingerprint=detail["fingerprint"].as_str().ok_or("propuesta sin huella")?.to_string();
                let id=format!("file:{fingerprint}");
                let mut offers=self.file_offers.lock().map_err(|_|"propuestas bloqueadas")?;
                if offers.get(&id).is_some_and(|item|item.granted) {
                    offers.remove(&id);
                    drop(offers);
                    if let Some(destination)=a.destination {
                        self.files.move_file(&a.path,&destination,&a.expected_sha256,&fingerprint)
                    } else {self.files.delete(&a.path,&a.expected_sha256,&fingerprint)}.map_err(|e|e.data().to_string())
                } else {
                    if offers.len()>=self.max_sessions { return Err("límite de operaciones propuestas alcanzado".into()); }
                    offers.insert(id.clone(),FileOffer{operation:operation.into(),path:a.path.clone(),destination:a.destination,sha256:a.expected_sha256,fingerprint,granted:false});
                    Ok(json!({"status":"awaiting_confirmation","path":a.path,"offer":id,"detail":detail,"error":"el usuario debe revisar y conceder esta operación en /permissions; no se ha ejecutado"}))
                }
            }),
            "list_files"|"read_file"=>serde_json::from_value::<PathArgs>(args).map_err(|e|e.to_string()).and_then(|a| {
                (if name=="list_files" {self.files.list(&a.path)} else {self.files.read(&a.path)}).map_err(|e|e.data().to_string())
            }),
            "search_files"=>serde_json::from_value::<SearchArgs>(args).map_err(|e|e.to_string()).and_then(|a|self.files.search(&a.path,&a.needle).map_err(|e|e.data().to_string())),
            "create_file"=>serde_json::from_value::<CreateArgs>(args).map_err(|e|e.to_string()).and_then(|a|self.files.create(&a.path,&a.content).map_err(|e|e.data().to_string())),
            "edit_file"=>serde_json::from_value::<EditArgs>(args).map_err(|e|e.to_string()).and_then(|a| {
                if self.text_observations.lock().map_err(|_|"registro de lecturas bloqueado")?.get(&a.path).is_none_or(|sha|!sha.eq_ignore_ascii_case(&a.expected_sha256)) {return Err("debes leer antes la misma ruta con read_file y usar su SHA256 observado".into());}
                self.files.edit(&a.path,&a.expected_sha256,&a.old,&a.new).map_err(|e|e.data().to_string())
            }),
            "run_command"=>match serde_json::from_value::<CommandArgs>(args) {
                Ok(a)=>self.run_command(&a.id,cancel).await,
                Err(e)=>Err(e.to_string()),
            },
            name if name.starts_with("mcp__") => match mcp::split_tool_name(name) {
                Some((server, tool)) => self.mcp.call(&self.trust, server, tool, args).await,
                None => Err("nombre de herramienta MCP no admitido".into()),
            },
            _=>Err("herramienta no disponible".into()),
        };
        let result = result.unwrap_or_else(|error| {
            serde_json::from_str(&error)
                .unwrap_or_else(|_| json!({"status":"failed","error":error}))
        });
        if matches!(
            result["status"].as_str(),
            Some("partial" | "unknown" | "applied")
        ) {
            self.uncertain_effects.store(true, Ordering::SeqCst);
        }
        if matches!(name, "read_file" | "inspect_path")
            && result["status"] == "verified"
            && let (Some(path), Some(sha)) = (result["path"].as_str(), result["sha256"].as_str())
        {
            if name == "read_file" {
                self.text_observations
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(path.into(), sha.into());
            }
            self.observations
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(path.into(), sha.into());
        }
        if matches!(name, "create_file" | "edit_file" | "edit_file_many")
            && result["status"] == "verified"
            && let (Some(path), Some(sha)) = (result["path"].as_str(), result["sha256"].as_str())
        {
            self.text_observations
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(path);
            self.versions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(path.into(), sha.into());
            self.observations
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(path);
        }
        if matches!(name, "move_file" | "delete_file")
            && result["status"] == "verified"
            && let Some(path) = result["path"].as_str()
        {
            self.text_observations
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(path);
            let mut versions = self.versions.lock().unwrap_or_else(|e| e.into_inner());
            versions.insert(path.into(), String::new());
            if let (Some(destination), Some(sha)) =
                (result["destination"].as_str(), result["sha256"].as_str())
            {
                versions.insert(destination.into(), sha.into());
            }
            self.observations
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(path);
        }
        if matches!(
            name,
            "create_file" | "edit_file" | "edit_file_many" | "move_file" | "delete_file"
        ) {
            self.retain_change(name, &result);
        }
        result
    }
}

pub struct Journal {
    file: File,
    pub path: PathBuf,
    pub id: String,
    bytes: usize,
    max: usize,
    sequence: u64,
    record_max: usize,
    _pins: Vec<File>,
    reserve: usize,
}
impl Journal {
    pub fn record(
        &mut self,
        role: Role,
        stage: &str,
        call: Option<&ToolCall>,
        result: Option<&Value>,
    ) -> Result<String, String> {
        self.write_record(role, stage, call, result, None, None)
    }
    pub fn requested(
        &mut self,
        role: Role,
        call: &ToolCall,
        target: Option<&str>,
    ) -> Result<String, String> {
        self.write_record(
            role,
            "requested",
            Some(call),
            None,
            None,
            Some(&json!({"target_sha256":target})),
        )
    }
    pub fn observed(
        &mut self,
        role: Role,
        call: &ToolCall,
        result: &Value,
        call_id: &str,
    ) -> Result<String, String> {
        self.write_record(
            role,
            "observed",
            Some(call),
            Some(result),
            Some(call_id),
            None,
        )
    }
    pub fn observed_read(
        &mut self,
        role: Role,
        call: &ToolCall,
        result: &Value,
        call_id: &str,
        execution: &Value,
    ) -> Result<String, String> {
        self.write_record(
            role,
            "observed",
            Some(call),
            Some(result),
            Some(call_id),
            Some(&json!({"execution":execution})),
        )
    }
    fn write_record(
        &mut self,
        role: Role,
        stage: &str,
        call: Option<&ToolCall>,
        result: Option<&Value>,
        call_id: Option<&str>,
        metadata: Option<&Value>,
    ) -> Result<String, String> {
        self.sequence += 1;
        let event_id = format!("{}-{}", self.id, self.sequence);
        let id = call_id.unwrap_or(&event_id);
        let hash = |value: &Value| {
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(value).unwrap_or_default())
            )
        };
        let args = call.map(|c| &c.function.arguments);
        let command = result.and_then(|r| r.get("command"));
        let value = json!({"session":self.id,"event":event_id,"call_id":id,"agent":role,"stage":stage,"tool":call.map(|c|&c.function.name),"arguments_sha256":args.map(hash),"path":args.and_then(|a|a.get("path")).or_else(||result.and_then(|r|r.get("path"))),"expected_sha256":args.and_then(|a|a.get("expected_sha256")),"command_id":args.and_then(|a|a.get("id")),"status":result.and_then(|r|r.get("status")),"result_sha256":result.map(hash),"backup_path":result.and_then(|r|r.get("backup_path")),"sha256":result.and_then(|r|r.get("sha256")),"base_sha256":result.and_then(|r|r.get("base_sha256")),"command":command.map(|c|json!({"id":c["id"],"fingerprint":c["fingerprint"],"exit_code":c["exit_code"],"timed_out":c["timed_out"],"cancelled":c["cancelled"],"elapsed_ms":c["elapsed_ms"],"stdout_sha256":hash(&c["stdout"]),"stderr_sha256":hash(&c["stderr"])}))});
        let mut value = value;
        value["effect_not_started"] = json!(
            call.is_some_and(|c| c.function.name.starts_with("mcp__"))
                && result.is_some_and(|r| r["status"] == "failed")
        );
        value["backup_sha256"] = result
            .and_then(|r| r.get("backup_sha256"))
            .or_else(|| args.and_then(|a| a.get("backup_sha256")))
            .cloned()
            .unwrap_or(Value::Null);
        if stage == "requested" {
            value["backup_path"] = args
                .and_then(|a| a.get("backup_path"))
                .cloned()
                .unwrap_or(Value::Null);
        }
        value["destination"] = args
            .and_then(|a| a.get("destination"))
            .or_else(|| result.and_then(|r| r.get("destination")))
            .cloned()
            .unwrap_or(Value::Null);
        value["target_sha256"] = metadata
            .and_then(|m| m.get("target_sha256"))
            .cloned()
            .unwrap_or(Value::Null);
        value["execution"] = metadata
            .and_then(|m| m.get("execution"))
            .cloned()
            .unwrap_or(Value::Null);
        let mut bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        if bytes.len() > self.record_max {
            return Err("registro excede agents.max_record_bytes; efectos posteriores no iniciados, revisar evidencia si ya observados".into());
        }
        let mut reserve = self.reserve;
        if stage == "requested" {
            // El máximo de paths publicados añade raíz y nombre de reserva a la ruta solicitada.
            let path_size = args
                .and_then(|a| a.get("path"))
                .map(|p| serde_json::to_vec(p).map(|b| b.len()).unwrap_or(0))
                .unwrap_or(0);
            if path_size
                .saturating_mul(2)
                .saturating_add(self.path.as_os_str().len().saturating_mul(4))
                .saturating_add(bytes.len())
                > self.record_max
            {
                return Err("ruta demasiado larga para auditoría configurada; sin ejecutar".into());
            }
            reserve = reserve.saturating_add(self.record_max);
        }
        if stage == "observed" {
            reserve = reserve.saturating_sub(self.record_max);
        }
        if self
            .bytes
            .saturating_add(bytes.len())
            .saturating_add(reserve)
            > self.max
        {
            return Err(format!("diario lleno: {}", self.path.display()));
        }
        self.file
            .write_all(&bytes)
            .and_then(|_| self.file.sync_data())
            .map_err(|e| {
                format!(
                    "diario {}: {e}; revisar efectos observados",
                    self.path.display()
                )
            })?;
        self.bytes += bytes.len();
        self.reserve = reserve;
        Ok(id.to_owned())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::providers::ollama::ToolFunction;

    fn call(name: &str, arguments: Value) -> ToolCall {
        ToolCall {
            function: ToolFunction {
                name: name.into(),
                arguments,
            },
        }
    }

    #[tokio::test]
    async fn successful_test_is_invalidated_by_changed_read_or_script_even_after_rereading() {
        for changed in ["data.txt", "check.ps1"] {
            let sandbox = tempfile::tempdir().unwrap();
            let root = sandbox.path().join("project");
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("data.txt"), "base").unwrap();
            std::fs::write(root.join("check.ps1"), "exit 0").unwrap();
            let mut config = Config::load(&root, None).unwrap().config;
            let program = std::env::split_paths(&std::env::var_os("PATH").unwrap())
                .map(|path| path.join("pwsh.exe"))
                .find(|path| path.is_file())
                .unwrap()
                .canonicalize()
                .unwrap();
            config.commands.commands = vec![commands::CommandSpec {
                id: "check".into(),
                program,
                args: vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-File".into(),
                    "check.ps1".into(),
                ],
                cwd: ".".into(),
                watched_files: vec!["check.ps1".into()],
            }];
            let tools = Toolset::new(
                &root,
                &config,
                None,
                &sandbox.path().join("private/trust.json"),
            )
            .unwrap();
            let runner = tools.commands.as_ref().unwrap();
            let proposal = runner.proposals().unwrap().remove(0);
            runner.grant("check", &proposal.fingerprint).unwrap();
            let cancel = Arc::new(AtomicBool::new(false));
            let read = call("read_file", json!({"path":"data.txt"}));
            assert_eq!(
                tools.execute(Role::Tester, &read, cancel.clone()).await["status"],
                "verified"
            );
            assert_eq!(
                tools
                    .execute(
                        Role::Tester,
                        &call("run_command", json!({"id":"check"})),
                        cancel.clone()
                    )
                    .await["status"],
                "verified"
            );
            tools.verify_task_evidence().unwrap();
            std::fs::write(
                root.join(changed),
                if changed == "check.ps1" {
                    "exit 1"
                } else {
                    "external"
                },
            )
            .unwrap();
            // Una nueva lectura del reviewer no reemplaza la base realmente probada.
            tools
                .execute(
                    Role::Reviewer,
                    &call("read_file", json!({"path":changed})),
                    cancel,
                )
                .await;
            assert!(tools.verify_task_evidence().is_err(), "{changed}");
        }
    }

    #[tokio::test]
    async fn partial_effect_stays_pending_across_new_requests_and_blocks_writes() {
        let sandbox = tempfile::tempdir().unwrap();
        let root = sandbox.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let config = Config::load(&root, None).unwrap().config;
        let tools = Toolset::new(
            &root,
            &config,
            None,
            &sandbox.path().join("private/trust.json"),
        )
        .unwrap();
        tools.uncertain_effects.store(true, Ordering::SeqCst);
        tools.reset_versions();
        let result = tools
            .execute(
                Role::Implementer,
                &call("create_file", json!({"path":"blocked.txt","content":"new"})),
                Arc::new(AtomicBool::new(false)),
            )
            .await;
        assert_eq!(result["status"], "failed");
        assert!(!root.join("blocked.txt").exists());
        assert!(
            tools
                .verify_task_evidence()
                .unwrap_err()
                .contains("parciales")
        );
    }

    #[test]
    fn exclusive_grant_and_revoke_round_trip_and_a_root_stays_refused() {
        let sandbox = tempfile::tempdir().unwrap();
        let root = sandbox.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let mut config = Config::load(&root, None).unwrap().config;
        config.gpu.exclusive = true;
        let tools = Toolset::new(
            &root,
            &config,
            None,
            &sandbox.path().join("private/trust.json"),
        )
        .unwrap();
        let fingerprint = crate::gpu::exclusive_fingerprint(&config.ollama.url);
        assert!(!tools.exclusive_granted().unwrap());
        tools.grant_offer("exclusive", &fingerprint).unwrap();
        assert!(tools.exclusive_granted().unwrap());
        tools.revoke_offer("exclusive").unwrap();
        assert!(!tools.exclusive_granted().unwrap());
        assert_eq!(
            tools.revoke_offer("root:0").unwrap_err(),
            "una raíz extra no tiene concesión que retirar"
        );
        assert_eq!(
            tools.grant_offer("root:0", "x").unwrap_err(),
            "una raíz extra no amplía el alcance"
        );
        assert!(
            tools
                .revoke_offer("desconocida")
                .unwrap_err()
                .contains("propuesta desconocida")
        );
    }
}
