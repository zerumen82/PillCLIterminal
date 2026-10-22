pub mod commands;
pub mod mcp;

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
            "read_file" | "list_files" | "search_files" => true,
            "create_file" | "edit_file" => self == Self::Implementer,
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
            "Lista archivos del directorio relativo. Usa . para raíz.",
            json!({"path":{"type":"string"}}),
            vec!["path"],
        ),
        (
            "read_file",
            "Lee texto UTF8 y SHA256 antes de editar.",
            json!({"path":{"type":"string"}}),
            vec!["path"],
        ),
        (
            "search_files",
            "Busca texto literal recursivamente; informa límites.",
            json!({"path":{"type":"string"},"needle":{"type":"string"}}),
            vec!["path", "needle"],
        ),
        (
            "create_file",
            "Crea sin sobrescribir; carpeta padre debe existir.",
            json!({"path":{"type":"string"},"content":{"type":"string"}}),
            vec!["path", "content"],
        ),
        (
            "edit_file",
            "Reemplaza una coincidencia exacta tras leer. Conserva backup.",
            json!({"path":{"type":"string"},"expected_sha256":{"type":"string"},"old":{"type":"string"},"new":{"type":"string"}}),
            vec!["path", "expected_sha256", "old", "new"],
        ),
        (
            "run_command",
            "Ejecuta solo id registrado y concedido por usuario.",
            json!({"id":{"type":"string"}}),
            vec!["id"],
        ),
    ];
    definitions.into_iter().filter(|(name,_,_,_)| role.permits(name) && (*name!="run_command" || commands_enabled)).map(|(name,description,properties,required)| json!({"type":"function","function":{"name":name,"description":description,"parameters":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}})).collect()
}

pub struct Toolset {
    pub files: Files,
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
    observations: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    tested: std::sync::Mutex<std::collections::BTreeMap<String, TestEvidence>>,
    uncertain_effects: AtomicBool,
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
        Ok(Self {
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
            observations: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            tested: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            uncertain_effects: AtomicBool::new(false),
        })
    }

    pub fn journal_byte_limit(&self) -> usize {
        self.max_journal_bytes
    }

    pub fn reset_versions(&self) {
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
            let read = self.files.read(path).map_err(|e| e.to_string())?;
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
            json!({"status":if verified {"verified"} else {"failed"},"command":result,"version_error":changed}),
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

    pub async fn execute(&self, role: Role, call: &ToolCall, cancel: Arc<AtomicBool>) -> Value {
        if cancel.load(Ordering::SeqCst) {
            return json!({"status":"cancelled","error":"cancelado antes de iniciar herramienta"});
        }
        let name = call.function.name.as_str();
        let effect = matches!(name, "create_file" | "edit_file" | "run_command")
            || name.starts_with("mcp__");
        if effect && self.uncertain_effects.load(Ordering::SeqCst) {
            return json!({"status":"failed","error":"efectos parciales pendientes; revisa diario y backups antes de reiniciar la sesión"});
        }
        if !role.permits(name) {
            return json!({"status":"failed","error":"herramienta desconocida o no concedida a este rol"});
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
        let args = call.function.arguments.clone();
        let result:Result<Value,String> = match name {
            "list_files"|"read_file"=>serde_json::from_value::<PathArgs>(args).map_err(|e|e.to_string()).and_then(|a| {
                (if name=="list_files" {self.files.list(&a.path)} else {self.files.read(&a.path)}).map_err(|e|e.data().to_string())
            }),
            "search_files"=>serde_json::from_value::<SearchArgs>(args).map_err(|e|e.to_string()).and_then(|a|self.files.search(&a.path,&a.needle).map_err(|e|e.data().to_string())),
            "create_file"=>serde_json::from_value::<CreateArgs>(args).map_err(|e|e.to_string()).and_then(|a|self.files.create(&a.path,&a.content).map_err(|e|e.data().to_string())),
            "edit_file"=>serde_json::from_value::<EditArgs>(args).map_err(|e|e.to_string()).and_then(|a| {
                if self.observations.lock().map_err(|_|"registro de lecturas bloqueado")?.get(&a.path).is_none_or(|sha|!sha.eq_ignore_ascii_case(&a.expected_sha256)) {return Err("debes leer antes la misma ruta con read_file y usar su SHA256 observado".into());}
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
        if name == "read_file"
            && result["status"] == "verified"
            && let (Some(path), Some(sha)) = (result["path"].as_str(), result["sha256"].as_str())
        {
            self.observations
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(path.into(), sha.into());
        }
        if matches!(name, "create_file" | "edit_file")
            && result["status"] == "verified"
            && let (Some(path), Some(sha)) = (result["path"].as_str(), result["sha256"].as_str())
        {
            self.versions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(path.into(), sha.into());
            self.observations
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(path);
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
        self.write_record(role, stage, call, result, None)
    }
    pub fn observed(
        &mut self,
        role: Role,
        call: &ToolCall,
        result: &Value,
        call_id: &str,
    ) -> Result<String, String> {
        self.write_record(role, "observed", Some(call), Some(result), Some(call_id))
    }
    fn write_record(
        &mut self,
        role: Role,
        stage: &str,
        call: Option<&ToolCall>,
        result: Option<&Value>,
        call_id: Option<&str>,
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
        let mut bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        if bytes.len() > self.record_max {
            return Err("registro excede agents.max_record_bytes; efectos posteriores no iniciados, revisar evidencia si ya observados".into());
        }
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
            self.reserve = self.record_max;
        }
        if self
            .bytes
            .saturating_add(bytes.len())
            .saturating_add(if stage == "observed" { 0 } else { self.reserve })
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
        if stage == "observed" {
            self.reserve = 0;
        }
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
