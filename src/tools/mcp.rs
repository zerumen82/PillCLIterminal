//! Cliente MCP stdio. Una propuesta no arranca el proceso hasta que el usuario la concede.

use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::process::Command;

use super::commands::CommandRunner;
use crate::config::{McpConfig, McpServerConfig};

#[derive(Debug)]
pub struct Offer {
    pub id: String,
    pub fingerprint: String,
    pub detail: String,
}

struct CachedSchemas {
    fingerprint: String,
    created: std::time::Instant,
    schemas: Vec<Value>,
}

pub struct Catalog {
    root: PathBuf,
    config: McpConfig,
    cache: std::sync::Mutex<std::collections::BTreeMap<String, CachedSchemas>>,
}

impl Catalog {
    pub fn new(root: &Path, config: McpConfig) -> Result<Self, String> {
        config.validate()?;
        Ok(Self {
            root: root.canonicalize().map_err(|e| e.to_string())?,
            config,
            cache: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        })
    }

    pub fn offers(&self, runner: &CommandRunner) -> Result<Vec<Offer>, String> {
        let grants = runner.load_grants().map_err(|e| e.to_string())?;
        self.config
            .servers
            .iter()
            .map(|server| {
                let prepared = prepare(&self.root, server, &self.config)?;
                let granted = grants
                    .mcp
                    .get(&self.root)
                    .and_then(|items| items.get(&server.id))
                    == Some(&prepared.fingerprint);
                Ok(Offer {
                    id: format!("mcp:{}", server.id),
                    fingerprint: prepared.fingerprint,
                    detail: serde_json::to_string(&json!({
                        "kind": "mcp",
                        "id": server.id,
                        "program": prepared.program,
                        "args": server.args,
                        "cwd": prepared.cwd,
                        "env": server.env,
                        "observed": prepared.observed,
                        "granted": granted
                    }))
                    .map_err(|e| e.to_string())?,
                })
            })
            .collect()
    }

    pub fn grant(&self, runner: &CommandRunner, id: &str, displayed: &str) -> Result<(), String> {
        let server = self
            .server(id)
            .ok_or_else(|| format!("servidor MCP no propuesto: {id}"))?;
        let prepared = prepare(&self.root, server, &self.config)?;
        if prepared.fingerprint != displayed {
            return Err(format!("la propuesta de {id} cambió; vuelve a mostrarla"));
        }
        let mut grants = runner.load_grants().map_err(|e| e.to_string())?;
        grants
            .mcp
            .entry(self.root.clone())
            .or_default()
            .insert(id.to_owned(), prepared.fingerprint);
        runner.save_grants(&grants).map_err(|e| e.to_string())
    }

    pub fn revoke(&self, runner: &CommandRunner, id: &str) -> Result<(), String> {
        let mut grants = runner.load_grants().map_err(|e| e.to_string())?;
        if let Some(items) = grants.mcp.get_mut(&self.root) {
            items.remove(id);
        }
        runner.save_grants(&grants).map_err(|e| e.to_string())
    }

    pub async fn schemas(&self, runner: &CommandRunner) -> (Vec<Value>, Vec<String>) {
        let mut schemas = Vec::new();
        let mut errors = Vec::new();
        for server in &self.config.servers {
            match self.published(runner, server).await {
                Ok(tools) => schemas.extend(tools),
                Err(error) => errors.push(format!("{}: {error}", server.id)),
            }
        }
        (schemas, errors)
    }

    pub async fn call(
        &self,
        runner: &CommandRunner,
        server_id: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        let server = self
            .server(server_id)
            .ok_or_else(|| format!("servidor MCP no propuesto: {server_id}"))?;
        if serde_json::to_vec(&arguments)
            .map_err(|e| e.to_string())?
            .len()
            > self.config.max_message_bytes
        {
            return Err("argumentos MCP superan mcp.max_message_bytes; sin enviar".into());
        }
        let prepared = self.granted(runner, server)?;
        let mut service = connect(server, &prepared, &self.config).await?;
        let outcome = async {
            let listed = list_tools(
                &service,
                self.config.call_timeout_ms,
                self.config.max_message_bytes,
            )
            .await?;
            let found = listed
                .into_iter()
                .find(|item| tool_name(item).ok().as_deref() == Some(tool))
                .ok_or_else(|| format!("el servidor no publicó {tool}"))?;
            let schema = tool_schema(&found)?;
            let validator = closed_schema(&schema, self.config.max_message_bytes)?;
            if !validator.is_valid(&arguments) {
                return Err("argumentos rechazados por el esquema de la herramienta".into());
            }
            let object = arguments.as_object().cloned().unwrap_or_default();
            let params =
                rmcp::model::CallToolRequestParams::new(tool.to_owned()).with_arguments(object);
            let result = tokio::time::timeout(
                Duration::from_millis(self.config.call_timeout_ms),
                service.call_tool(params),
            )
            .await
            .map_err(|_| unknown("el servidor MCP no respondió a tiempo después del envío"))?
            .map_err(|error| unknown(&format!("llamada MCP sin confirmación: {error}")))?;
            let value =
                serde_json::to_value(&result).map_err(|error| unknown(&error.to_string()))?;
            if serde_json::to_vec(&value)
                .map_err(|e| unknown(&e.to_string()))?
                .len()
                > self.config.max_message_bytes
            {
                return Err(unknown(
                    "resultado MCP supera mcp.max_message_bytes después del envío",
                ));
            }
            if value.get("isError").and_then(Value::as_bool) == Some(true)
                || value.get("is_error").and_then(Value::as_bool) == Some(true)
            {
                return Err(unknown(&format!(
                    "herramienta MCP con error después del envío: {}",
                    texts(&value)
                )));
            }
            Ok(json!({
                "status": "verified",
                "server": server_id,
                "tool": tool,
                "text": texts(&value)
            }))
        }
        .await;
        service
            .close_with_timeout(Duration::from_millis(self.config.shutdown_timeout_ms))
            .await
            .map_err(|e| unknown(&format!("no se confirmó cierre MCP: {e}")))?;
        outcome
    }

    fn server(&self, id: &str) -> Option<&McpServerConfig> {
        self.config.servers.iter().find(|server| server.id == id)
    }

    fn granted(
        &self,
        runner: &CommandRunner,
        server: &McpServerConfig,
    ) -> Result<Prepared, String> {
        let prepared = prepare(&self.root, server, &self.config)?;
        let grants = runner.load_grants().map_err(|error| error.to_string())?;
        if grants
            .mcp
            .get(&self.root)
            .and_then(|items| items.get(&server.id))
            != Some(&prepared.fingerprint)
        {
            return Err(format!(
                "el usuario debe conceder el servidor {}",
                server.id
            ));
        }
        Ok(prepared)
    }

    async fn published(
        &self,
        runner: &CommandRunner,
        server: &McpServerConfig,
    ) -> Result<Vec<Value>, String> {
        let prepared = match self.granted(runner, server) {
            Ok(prepared) => prepared,
            Err(error) if error.contains("debe conceder") => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        if let Some(cached) = self
            .cache
            .lock()
            .map_err(|_| "catálogo MCP bloqueado")?
            .get(&server.id)
            && cached.fingerprint == prepared.fingerprint
            && cached.created.elapsed() < Duration::from_millis(self.config.catalog_cache_ms)
        {
            return Ok(cached.schemas.clone());
        }
        let mut service = connect(server, &prepared, &self.config).await?;
        let outcome = async {
        let tools = list_tools(&service, self.config.call_timeout_ms, self.config.max_message_bytes).await?;
        let mut schemas = Vec::new();
        for tool in &tools {
            let name = tool_name(tool)?;
            if !valid_token(&name) {
                return Err(format!("{name} no es un nombre de herramienta admitido"));
            }
            let schema = tool_schema(tool)?;
            closed_schema(&schema, self.config.max_message_bytes)?;
            schemas.push(json!({
                "type": "function",
                "function": {
                    "name": format!("mcp__{}__{name}", server.id),
                    "description": format!("Herramienta del servidor MCP concedido {}. Se ejecuta con tus permisos de Windows. Una anotación de solo lectura no autoriza nada.", server.id),
                    "parameters": schema
                }
            }));
        }
        Ok(schemas)
        }.await;
        service
            .close_with_timeout(Duration::from_millis(self.config.shutdown_timeout_ms))
            .await
            .map_err(|e| format!("cierre MCP no confirmado: {e}"))?;
        if let Ok(schemas) = &outcome {
            self.cache
                .lock()
                .map_err(|_| "catálogo MCP bloqueado")?
                .insert(
                    server.id.clone(),
                    CachedSchemas {
                        fingerprint: prepared.fingerprint,
                        created: std::time::Instant::now(),
                        schemas: schemas.clone(),
                    },
                );
        }
        outcome
    }
}

struct Prepared {
    program: PathBuf,
    cwd: PathBuf,
    fingerprint: String,
    observed: Vec<String>,
    _locks: Vec<File>,
}

fn prepare(root: &Path, server: &McpServerConfig, config: &McpConfig) -> Result<Prepared, String> {
    super::commands::reject_links(&root.join(&server.cwd)).map_err(|e| e.to_string())?;
    super::commands::reject_links(&server.program).map_err(|e| e.to_string())?;
    let cwd = root
        .join(&server.cwd)
        .canonicalize()
        .map_err(|error| format!("cwd: {error}"))?;
    if !cwd.starts_with(root) || !cwd.is_dir() {
        return Err("cwd MCP fuera de la raíz o no es un directorio".into());
    }
    let program = server
        .program
        .canonicalize()
        .map_err(|error| format!("programa: {error}"))?;
    if !program.is_file() {
        return Err("el programa MCP no es un archivo".into());
    }
    let mut observed = Vec::new();
    let mut locks = Vec::new();
    super::commands::pin_directories(&cwd, &mut locks).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    hash.update(
        serde_json::to_vec(&(
            root,
            &server.id,
            &server.program,
            &server.args,
            &server.cwd,
            &server.env,
            config.startup_timeout_ms,
            config.call_timeout_ms,
            config.max_message_bytes,
            config.shutdown_timeout_ms,
            config.shutdown_poll_ms,
        ))
        .map_err(|error| error.to_string())?,
    );
    hash_file(&program, &mut hash, &mut observed, &mut locks)?;
    let mut saw_script = false;
    for arg in &server.args {
        let path = Path::new(arg);
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else if !arg.starts_with('-') {
            cwd.join(path)
        } else {
            continue;
        };
        if !candidate.is_file() {
            continue;
        }
        super::commands::reject_links(&candidate).map_err(|e| e.to_string())?;
        let candidate = candidate
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !candidate.starts_with(root) {
            return Err("un argumento de archivo MCP queda fuera de la raíz".into());
        }
        hash_file(&candidate, &mut hash, &mut observed, &mut locks)?;
        saw_script = true;
    }
    if !saw_script {
        return Err(format!(
            "{}: incluye el script dentro de cwd; si cambia, la concesión deja de valer",
            server.id
        ));
    }
    for lock in &locks {
        super::commands::check_metadata_link(&lock.metadata().map_err(|e| e.to_string())?, &cwd)
            .map_err(|e| e.to_string())?;
    }
    Ok(Prepared {
        program,
        cwd,
        fingerprint: format!("{:x}", hash.finalize()),
        observed,
        _locks: locks,
    })
}

fn hash_file(
    path: &Path,
    hash: &mut Sha256,
    observed: &mut Vec<String>,
    locks: &mut Vec<File>,
) -> Result<(), String> {
    super::commands::pin_directories(path.parent().ok_or("archivo sin padre")?, locks)
        .map_err(|e| e.to_string())?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        };
        options
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    super::commands::check_metadata_link(&file.metadata().map_err(|e| e.to_string())?, path)
        .map_err(|e| e.to_string())?;
    super::commands::verify_opened_path(&file, path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(16_777_217)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 16_777_216 {
        return Err(format!("{} supera el tamaño observado", path.display()));
    }
    hash.update(path.to_string_lossy().as_bytes());
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(&bytes);
    let mut file_hash = Sha256::new();
    file_hash.update(&bytes);
    observed.push(format!("{} {:x}", path.display(), file_hash.finalize()));
    locks.push(file);
    Ok(())
}

fn closed_schema(schema: &Value, max_bytes: usize) -> Result<jsonschema::Validator, String> {
    let bytes = serde_json::to_vec(schema).map_err(|error| error.to_string())?;
    if bytes.len() > max_bytes {
        return Err("esquema MCP demasiado grande".into());
    }
    if external_ref(schema) {
        return Err(
            "el esquema contiene una referencia externa; no se resuelve por red ni leyendo archivos"
                .into(),
        );
    }
    jsonschema::options()
        .offline()
        .build(schema)
        .map_err(|error| format!("esquema MCP malformado: {error}"))
}

fn external_ref(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            if let Some(reference) = map.get("$ref").and_then(Value::as_str)
                && !reference.starts_with('#')
            {
                return true;
            }
            map.values().any(external_ref)
        }
        Value::Array(items) => items.iter().any(external_ref),
        _ => false,
    }
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn tool_name(tool: &impl serde::Serialize) -> Result<String, String> {
    let value = serde_json::to_value(tool).map_err(|error| error.to_string())?;
    value
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "la herramienta MCP no trae nombre".into())
}

fn tool_schema(tool: &impl serde::Serialize) -> Result<Value, String> {
    let value = serde_json::to_value(tool).map_err(|error| error.to_string())?;
    value
        .get("inputSchema")
        .or_else(|| value.get("input_schema"))
        .cloned()
        .ok_or_else(|| "la herramienta MCP no trae esquema".into())
}

fn texts(value: &Value) -> String {
    value
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

type Client = rmcp::service::RunningService<rmcp::RoleClient, ()>;

fn unknown(error: &str) -> String {
    json!({"status":"unknown","error":error,"recovery":"MCP pudo producir efectos; revisar diario antes de más cambios"}).to_string()
}

async fn list_tools(
    service: &Client,
    timeout_ms: u64,
    max_bytes: usize,
) -> Result<Vec<rmcp::model::Tool>, String> {
    let tools = tokio::time::timeout(Duration::from_millis(timeout_ms), async {
        let mut tools = Vec::new();
        let mut cursor = None;
        let mut seen = std::collections::BTreeSet::new();
        let mut total = 0usize;
        loop {
            let page = service
                .list_tools(Some(
                    rmcp::model::PaginatedRequestParams::default().with_cursor(cursor),
                ))
                .await
                .map_err(|e| format!("listado MCP fallido: {e}"))?;
            total =
                total.saturating_add(serde_json::to_vec(&page).map_err(|e| e.to_string())?.len());
            if total > max_bytes {
                return Err("listado MCP supera mcp.max_message_bytes".to_string());
            }
            tools.extend(page.tools);
            cursor = page.next_cursor;
            if let Some(value) = &cursor {
                if !seen.insert(value.clone()) {
                    return Err("cursor MCP repetido".into());
                }
            } else {
                break;
            }
        }
        Ok(tools)
    })
    .await
    .map_err(|_| {
        "el servidor MCP no respondió a tiempo; la llamada no es un éxito".to_string()
    })??;
    if serde_json::to_vec(&tools).map_err(|e| e.to_string())?.len() > max_bytes {
        return Err("listado MCP supera mcp.max_message_bytes".into());
    }
    Ok(tools)
}

async fn connect(
    server: &McpServerConfig,
    prepared: &Prepared,
    config: &McpConfig,
) -> Result<Client, String> {
    let mut command = Command::new(&prepared.program);
    command
        .args(&server.args)
        .current_dir(&prepared.cwd)
        .kill_on_drop(true);
    for entry in &server.env {
        if let Some((key, value)) = entry.split_once('=') {
            command.env(key, value);
        }
    }
    #[cfg(windows)]
    command.creation_flags(
        windows_sys::Win32::System::Threading::CREATE_NO_WINDOW
            | windows_sys::Win32::System::Threading::CREATE_SUSPENDED,
    );
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let job = super::process_job::OwnedJob::assign(&child).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    super::process_job::resume(&child).map_err(|e| e.to_string())?;
    let input = child.stdin.take().ok_or("MCP sin stdin")?;
    let output = child.stdout.take().ok_or("MCP sin stdout")?;
    let transport = BoundedChild {
        #[cfg(windows)]
        job,
        shutdown_timeout_ms: config.shutdown_timeout_ms,
        shutdown_poll_ms: config.shutdown_poll_ms,
        inner: rmcp::transport::async_rw::AsyncRwTransport::new(output, input)
            .with_max_line_length(config.max_message_bytes),
        child,
        _pins: prepared
            ._locks
            .iter()
            .map(File::try_clone)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?,
    };
    tokio::time::timeout(
        Duration::from_millis(config.startup_timeout_ms),
        <() as rmcp::ServiceExt<rmcp::RoleClient>>::serve((), transport),
    )
    .await
    .map_err(|_| "el servidor MCP no completó el inicio".to_string())?
    .map_err(|error| format!("inicio MCP: {error}"))
}

struct BoundedChild {
    #[cfg(windows)]
    job: super::process_job::OwnedJob,
    shutdown_timeout_ms: u64,
    shutdown_poll_ms: u64,
    inner: rmcp::transport::async_rw::AsyncRwTransport<
        rmcp::RoleClient,
        tokio::process::ChildStdout,
        tokio::process::ChildStdin,
    >,
    child: tokio::process::Child,
    _pins: Vec<File>,
}

impl rmcp::transport::Transport<rmcp::RoleClient> for BoundedChild {
    type Error = std::io::Error;
    fn send(
        &mut self,
        item: rmcp::service::TxJsonRpcMessage<rmcp::RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(item)
    }
    fn receive(
        &mut self,
    ) -> impl Future<Output = Option<rmcp::service::RxJsonRpcMessage<rmcp::RoleClient>>> + Send
    {
        self.inner.receive()
    }
    async fn close(&mut self) -> Result<(), Self::Error> {
        let _ = self.inner.close().await;
        #[cfg(windows)]
        {
            self.job.terminate()?;
            let deadline =
                std::time::Instant::now() + Duration::from_millis(self.shutdown_timeout_ms);
            while self.job.active()? > 0 {
                if std::time::Instant::now() >= deadline {
                    return Err(std::io::Error::other("el árbol MCP no confirmó su cierre"));
                }
                tokio::time::sleep(Duration::from_millis(self.shutdown_poll_ms)).await;
            }
        }
        if self.child.try_wait()?.is_none() {
            self.child.kill().await?;
        }
        Ok(())
    }
}

pub fn split_tool_name(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    if valid_token(server) && valid_token(tool) {
        Some((server, tool))
    } else {
        None
    }
}
