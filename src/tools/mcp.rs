//! Cliente MCP stdio. Una propuesta no arranca el proceso hasta que el usuario la concede.

use std::{
    fs::File,
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

pub struct Catalog {
    root: PathBuf,
    config: McpConfig,
}

impl Catalog {
    pub fn new(root: &Path, config: McpConfig) -> Result<Self, String> {
        config.validate()?;
        Ok(Self {
            root: root.canonicalize().map_err(|e| e.to_string())?,
            config,
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
        let prepared = self.granted(runner, server)?;
        let service = connect(server, &prepared, &self.config).await?;
        let listed = list_tools(&service, self.config.call_timeout_ms).await?;
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
        .map_err(|_| {
            "el servidor MCP no respondió a tiempo; la llamada no es un éxito".to_string()
        })?
        .map_err(|error| format!("llamada MCP fallida: {error}"))?;
        let value = serde_json::to_value(&result).map_err(|error| error.to_string())?;
        if value.get("isError").and_then(Value::as_bool) == Some(true)
            || value.get("is_error").and_then(Value::as_bool) == Some(true)
        {
            return Err(format!("herramienta MCP con error: {}", texts(&value)));
        }
        Ok(json!({
            "status": "verified",
            "server": server_id,
            "tool": tool,
            "text": texts(&value)
        }))
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
        let service = connect(server, &prepared, &self.config).await?;
        let tools = list_tools(&service, self.config.call_timeout_ms).await?;
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
    }
}

struct Prepared {
    program: PathBuf,
    cwd: PathBuf,
    fingerprint: String,
    observed: Vec<String>,
}

fn prepare(root: &Path, server: &McpServerConfig, config: &McpConfig) -> Result<Prepared, String> {
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
        ))
        .map_err(|error| error.to_string())?,
    );
    hash_file(&program, &mut hash, &mut observed)?;
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
        let candidate = candidate
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !candidate.starts_with(root) {
            return Err("un argumento de archivo MCP queda fuera de la raíz".into());
        }
        hash_file(&candidate, &mut hash, &mut observed)?;
        saw_script = true;
    }
    if !saw_script {
        return Err(format!(
            "{}: incluye el script dentro de cwd; si cambia, la concesión deja de valer",
            server.id
        ));
    }
    Ok(Prepared {
        program,
        cwd,
        fingerprint: format!("{:x}", hash.finalize()),
        observed,
    })
}

fn hash_file(path: &Path, hash: &mut Sha256, observed: &mut Vec<String>) -> Result<(), String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
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

async fn list_tools(service: &Client, timeout_ms: u64) -> Result<Vec<rmcp::model::Tool>, String> {
    tokio::time::timeout(Duration::from_millis(timeout_ms), service.list_all_tools())
        .await
        .map_err(|_| {
            "el servidor MCP no respondió a tiempo; la llamada no es un éxito".to_string()
        })?
        .map_err(|error| format!("listado MCP fallido: {error}"))
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
    command.creation_flags(0x0800_0000);
    let (transport, _stderr) = rmcp::transport::TokioChildProcess::builder(command)
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    tokio::time::timeout(
        Duration::from_millis(config.startup_timeout_ms),
        <() as rmcp::ServiceExt<rmcp::RoleClient>>::serve((), transport),
    )
    .await
    .map_err(|_| "el servidor MCP no completó el inicio".to_string())?
    .map_err(|error| format!("inicio MCP: {error}"))
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
