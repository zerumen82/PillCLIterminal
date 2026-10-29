use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Error;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub ui: UiConfig,
    pub ollama: OllamaConfig,
    pub files: crate::files::FilesConfig,
    pub commands: crate::tools::commands::CommandsConfig,
    pub agents: AgentConfig,
    pub mcp: McpConfig,
    pub gpu: crate::gpu::GpuConfig,
    pub profiles: std::collections::BTreeMap<String, OllamaProfile>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfig {
    pub id: String,
    pub program: std::path::PathBuf,
    pub args: Vec<String>,
    pub cwd: std::path::PathBuf,
    #[serde(default)]
    pub env: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    pub startup_timeout_ms: u64,
    pub call_timeout_ms: u64,
    pub max_message_bytes: usize,
    pub shutdown_timeout_ms: u64,
    pub shutdown_poll_ms: u64,
    pub catalog_cache_ms: u64,
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

impl McpConfig {
    pub fn validate(&self) -> Result<(), String> {
        use std::collections::BTreeSet;
        use std::path::Component;
        if !(100..=60_000).contains(&self.startup_timeout_ms)
            || !(100..=600_000).contains(&self.call_timeout_ms)
            || !(1024..=16_777_216).contains(&self.max_message_bytes)
            || self.startup_timeout_ms > self.call_timeout_ms
            || !(100..=30_000).contains(&self.shutdown_timeout_ms)
            || !(1..=1000).contains(&self.shutdown_poll_ms)
            || self.shutdown_poll_ms > self.shutdown_timeout_ms
            || self.catalog_cache_ms > 300_000
            || self.servers.len() > 16
        {
            return Err("límites de mcp fuera de rango".into());
        }
        let mut ids = BTreeSet::new();
        for server in &self.servers {
            let id_ok = !server.id.is_empty()
                && server.id.len() <= 64
                && server
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !id_ok || server.id.contains("__") || !ids.insert(server.id.as_str()) {
                return Err("id MCP vacío, repetido o con caracteres no admitidos".into());
            }
            if !server.program.is_absolute()
                || server.args.len() > 64
                || server.env.len() > 32
                || server
                    .args
                    .iter()
                    .chain(&server.env)
                    .any(|value| value.contains('\0') || value.len() > 4096)
                || server.cwd.is_absolute()
                || server
                    .cwd
                    .components()
                    .any(|component| matches!(component, Component::ParentDir))
            {
                return Err(format!(
                    "{}: programa absoluto, cwd relativo y argumentos sin NUL requeridos",
                    server.id
                ));
            }
            for entry in &server.env {
                let Some((key, _)) = entry.split_once('=') else {
                    return Err(format!(
                        "{}: el entorno explícito usa CLAVE=valor",
                        server.id
                    ));
                };
                let mut chars = key.chars();
                let ok = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
                    && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !ok {
                    return Err(format!("{key} no es una clave de entorno admitida"));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub max_parallel_reads: usize,
    pub max_steps: usize,
    pub max_tool_calls: usize,
    pub max_handoff_chars: usize,
    pub max_journal_bytes: usize,
    pub max_record_bytes: usize,
    pub max_sessions: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OllamaConfig {
    pub url: String,
    pub model: Option<String>,
    pub num_ctx: u32,
    pub num_predict: u32,
    pub temperature: f64,
    pub think: bool,
    pub connect_timeout_ms: u64,
    pub response_timeout_ms: u64,
    pub max_metadata_bytes: usize,
    pub max_line_bytes: usize,
    pub max_response_bytes: usize,
    pub system_prompt: String,
    pub event_buffer: usize,
    pub metadata_retries: u32,
    pub retry_delay_ms: u64,
    pub first_chunk_timeout_ms: u64,
    pub idle_timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OllamaProfile {
    pub num_ctx: u32,
    pub num_predict: u32,
    pub temperature: f64,
    pub think: bool,
}

impl OllamaProfile {
    pub fn apply(&self, config: &OllamaConfig) -> Result<OllamaConfig, String> {
        if !(512..=131072).contains(&self.num_ctx)
            || self.num_predict == 0
            || self.num_predict >= self.num_ctx / 2
            || !(0.0..=2.0).contains(&self.temperature)
            || config.system_prompt.len() > self.num_ctx as usize / 2
        {
            return Err(
                "perfil inválido: contexto, generación o temperatura fuera de rango".into(),
            );
        }
        let mut configured = config.clone();
        configured.num_ctx = self.num_ctx;
        configured.num_predict = self.num_predict;
        configured.temperature = self.temperature;
        configured.think = self.think;
        Ok(configured)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    pub ascii_logo: bool,
    pub color: bool,
    pub diff_context_lines: usize,
    pub diff_max_cells: usize,
    pub logo_color: LogoColor,
    pub poll_ms: u32,
    pub max_input_chars: usize,
    pub max_history_entries: usize,
    pub compact_header: bool,
    pub max_history_bytes: usize,
    pub max_detail_entries: usize,
    pub render_interval_ms: u64,
    pub max_input_rows: usize,
    pub reserve_percent_warning: usize,
    pub reserve_percent_critical: usize,
    pub reserve_color_warning: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogoColor {
    Green,
    Cyan,
    White,
}

pub struct LoadedConfig {
    pub config: Config,
    pub source: Option<PathBuf>,
}

impl Config {
    pub fn load(project: &Path, explicit: Option<&Path>) -> Result<LoadedConfig, Error> {
        let mut value: Value = serde_json::from_str(include_str!("../../config/defaults.json"))?;
        let path = explicit
            .map(|p| {
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    project.join(p)
                }
            })
            .unwrap_or_else(|| project.join(".pillcli/config.json"));
        let source = match fs::read_to_string(&path) {
            Ok(text) => {
                let patch: Value = serde_json::from_str(&text)
                    .map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
                merge(&mut value, patch);
                Some(path)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && explicit.is_none() => {
                None
            }
            Err(error) => return Err(Error::Config(format!("{}: {error}", path.display()))),
        };
        let config: Self = serde_json::from_value(value)?;
        config.validate()?;
        Ok(LoadedConfig { config, source })
    }

    fn validate(&self) -> Result<(), Error> {
        self.files
            .validate()
            .map_err(|e| Error::Config(e.to_string()))?;
        self.commands
            .validate()
            .map_err(|e| Error::Config(e.to_string()))?;
        self.mcp.validate().map_err(Error::Config)?;
        self.gpu.validate().map_err(Error::Config)?;
        if !(1..=8).contains(&self.agents.max_parallel_reads)
            || !(1..=64).contains(&self.agents.max_steps)
            || !(1..=32).contains(&self.agents.max_tool_calls)
            || !(256..=16384).contains(&self.agents.max_handoff_chars)
            || !(4096..=16777216).contains(&self.agents.max_journal_bytes)
            || !(4096..=1048576).contains(&self.agents.max_record_bytes)
            || self.agents.max_record_bytes.saturating_mul(2) > self.agents.max_journal_bytes
            || !(1..=10000).contains(&self.agents.max_sessions)
        {
            return Err(Error::Config("límites inválidos en agents".into()));
        }
        crate::providers::ollama::local_url(&self.ollama.url)
            .map_err(|e| Error::Config(e.to_string()))?;
        let o = &self.ollama;
        if !(512..=131072).contains(&o.num_ctx)
            || !(2..=1024).contains(&o.event_buffer)
            || o.num_predict == 0
            || o.num_predict >= o.num_ctx / 2
            || !(0.0..=2.0).contains(&o.temperature)
            || !(100..=30000).contains(&o.connect_timeout_ms)
            || !(1000..=600000).contains(&o.response_timeout_ms)
            || o.response_timeout_ms < o.connect_timeout_ms
            || o.metadata_retries > 5
            || !(1..=5000).contains(&o.retry_delay_ms)
            || !(100..=600000).contains(&o.first_chunk_timeout_ms)
            || !(100..=600000).contains(&o.idle_timeout_ms)
            || !(1024..=16777216).contains(&o.max_metadata_bytes)
            || !(1024..=1048576).contains(&o.max_line_bytes)
            || !(o.max_line_bytes..=16777216).contains(&o.max_response_bytes)
            || o.system_prompt.is_empty()
            || o.system_prompt.len() > o.num_ctx as usize / 2
            || o.model
                .as_ref()
                .is_some_and(|m| m.trim().is_empty() || m.chars().any(char::is_control))
        {
            return Err(Error::Config(
                "límites inválidos en ollama; revisa contexto, generación, tiempos y tamaños"
                    .into(),
            ));
        }
        if !(10..=5000).contains(&self.ui.poll_ms) {
            return Err(Error::Config(
                "ui.poll_ms debe estar entre 10 y 5000".into(),
            ));
        }
        if !(1..=65536).contains(&self.ui.max_input_chars) {
            return Err(Error::Config(
                "ui.max_input_chars debe estar entre 1 y 65536".into(),
            ));
        }
        if !(1..=4096).contains(&self.ui.max_history_entries) {
            return Err(Error::Config(
                "ui.max_history_entries debe estar entre 1 y 4096".into(),
            ));
        }
        if !(4096..=16777216).contains(&self.ui.max_history_bytes)
            || !(1..=1024).contains(&self.ui.max_detail_entries)
            || !(10..=1000).contains(&self.ui.render_interval_ms)
            || !(1..=32).contains(&self.ui.max_input_rows)
            || self.ui.diff_context_lines > 16
            || !(1..=4194304).contains(&self.ui.diff_max_cells)
            || !(0..=100).contains(&self.ui.reserve_percent_warning)
            || !(0..=100).contains(&self.ui.reserve_percent_critical)
            || self.ui.reserve_percent_warning > self.ui.reserve_percent_critical
        {
            return Err(Error::Config(
                "límites inválidos de historial, detalle o render en ui".into(),
            ));
        }
        if self.profiles.len() > 16 {
            return Err(Error::Config("como máximo 16 perfiles".into()));
        }
        for (name, profile) in &self.profiles {
            if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
                return Err(Error::Config("nombre de perfil vacío o inválido".into()));
            }
            profile.apply(o).map_err(Error::Config)?;
        }
        Ok(())
    }
}

fn merge(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                if let Some(current) = base.get_mut(&key) {
                    merge(current, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, patch) => *base = patch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_preferences_preserve_limits_and_reject_grants() {
        let mut value: Value =
            serde_json::from_str(include_str!("../../config/defaults.json")).unwrap();
        merge(&mut value, serde_json::json!({"ui":{"ascii_logo":true}}));
        let config: Config = serde_json::from_value(value.clone()).unwrap();
        assert!(config.ui.ascii_logo);
        assert!(config.ui.max_input_chars > 0);
        merge(
            &mut value,
            serde_json::json!({"trusted_commands":["delete"]}),
        );
        assert!(serde_json::from_value::<Config>(value).is_err());
    }

    #[test]
    fn unbounded_input_and_busy_polling_are_rejected() {
        let mut config: Config =
            serde_json::from_str(include_str!("../../config/defaults.json")).unwrap();
        config.ui.poll_ms = 0;
        assert!(config.validate().is_err());
        config.ui.poll_ms = 250;
        config.ui.max_input_chars = usize::MAX;
        assert!(config.validate().is_err());
    }
}
