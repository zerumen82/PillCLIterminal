//! Liberación de modelos propios. No mata procesos ni atribuye `/api/ps`.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GpuConfig {
    pub exclusive: bool,
    pub nvidia_smi: Option<PathBuf>,
}

impl GpuConfig {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(path) = &self.nvidia_smi
            && (!path.is_absolute()
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir)))
        {
            return Err("gpu.nvidia_smi debe ser una ruta absoluta sin ..".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct RunningModel {
    pub name: String,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleasePlan {
    pub unload: Vec<String>,
    pub conserved: Vec<String>,
    pub reason: String,
}

pub fn exclusive_fingerprint(url: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"pill-exclusive-v1\n");
    hash.update(url.as_bytes());
    format!("{:x}", hash.finalize())
}

/// `exclusive` solo es cierto cuando la configuración lo propone y el usuario
/// concedió la huella vigente. Los modelos ajenos nunca entran en `unload`.
pub fn plan(exclusive: bool, owned: &BTreeSet<String>, running: &[RunningModel]) -> ReleasePlan {
    if !exclusive {
        return ReleasePlan {
            unload: Vec::new(),
            conserved: running.iter().map(|model| model.name.clone()).collect(),
            reason: "servidor compartido o exclusividad no concedida; /api/ps no identifica al cliente y no se descarga nada".into(),
        };
    }
    let mut unload = Vec::new();
    let mut conserved = Vec::new();
    for model in running {
        if owned.contains(&model.name) {
            unload.push(model.name.clone());
        } else {
            conserved.push(model.name.clone());
        }
    }
    let reason = if unload.is_empty() {
        "exclusividad concedida; no hay modelos de esta sesión en memoria. El resto se conserva porque su propietario es desconocido".into()
    } else if conserved.is_empty() {
        "exclusividad concedida; se solicita la descarga solo de los modelos que esta sesión seleccionó".into()
    } else {
        "exclusividad concedida; se descargan los modelos de esta sesión y se conservan los de propietario desconocido".into()
    };
    ReleasePlan {
        unload,
        conserved,
        reason,
    }
}

pub fn nvidia_line(configured: Option<&Path>) -> String {
    let path = match configured {
        Some(path) => path.to_path_buf(),
        None => PathBuf::from(r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe"),
    };
    if !path.is_file() {
        return "medición NVIDIA no disponible".into();
    }
    match Command::new(&path)
        .args([
            "--query-gpu=name,memory.total,memory.used,memory.free",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            let line = text.lines().next().unwrap_or("").trim();
            if line.is_empty() {
                "medición NVIDIA no disponible".into()
            } else {
                format!("NVIDIA: {line}")
            }
        }
        _ => "medición NVIDIA no disponible".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_server_keeps_every_loaded_model() {
        let running = vec![RunningModel {
            name: "ajeno".into(),
            digest: "abc".into(),
        }];
        let plan = plan(false, &BTreeSet::from(["propio".into()]), &running);
        assert!(plan.unload.is_empty());
        assert_eq!(plan.conserved, vec!["ajeno".to_string()]);
    }

    #[test]
    fn exclusive_unloads_only_names_selected_by_this_session() {
        let running = vec![
            RunningModel {
                name: "propio".into(),
                digest: "1".into(),
            },
            RunningModel {
                name: "ajeno".into(),
                digest: "2".into(),
            },
        ];
        let plan = plan(true, &BTreeSet::from(["propio".into()]), &running);
        assert_eq!(plan.unload, vec!["propio".to_string()]);
        assert_eq!(plan.conserved, vec!["ajeno".to_string()]);
    }

    #[test]
    fn missing_nvidia_binary_does_not_invent_memory() {
        let missing = PathBuf::from(r"D:\no-existe\nvidia-smi.exe");
        assert_eq!(nvidia_line(Some(&missing)), "medición NVIDIA no disponible");
    }
}
