use std::{ffi::OsString, path::PathBuf};

use crate::Error;

#[derive(Debug, PartialEq, Eq)]
pub enum Options {
    Help,
    Version,
    Launch {
        project: Option<PathBuf>,
        config: Option<PathBuf>,
        model: Option<String>,
    },
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, Error> {
        let mut args = args.into_iter();
        let (mut project, mut config) = (None, None);
        let mut model = None;
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--help" | "-h") => return Ok(Self::Help),
                Some("--version" | "-V") => return Ok(Self::Version),
                Some("--model") => {
                    let value = args
                        .next()
                        .and_then(|s| s.into_string().ok())
                        .filter(|s| {
                            !s.trim().is_empty()
                                && !s.starts_with("--")
                                && !s.chars().any(char::is_control)
                        })
                        .ok_or_else(|| {
                            Error::Args("Falta un nombre UTF-8 válido para --model".into())
                        })?;
                    if model.replace(value).is_some() {
                        return Err(Error::Args("Opción repetida: --model".into()));
                    }
                }
                Some("--project" | "--config") => {
                    let label = arg.to_string_lossy();
                    let value = args
                        .next()
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| Error::Args(format!("Falta el valor de {label}")))?;
                    if value.to_string_lossy().starts_with("--") {
                        return Err(Error::Args(format!("Falta el valor de {label}")));
                    }
                    let target = if label == "--project" {
                        &mut project
                    } else {
                        &mut config
                    };
                    if target.replace(PathBuf::from(value)).is_some() {
                        return Err(Error::Args(format!("Opción repetida: {label}")));
                    }
                }
                _ => {
                    return Err(Error::Args(format!(
                        "Opción desconocida: {}. Consulta pill --help.",
                        arg.to_string_lossy()
                    )));
                }
            }
        }
        Ok(Self::Launch {
            project,
            config,
            model,
        })
    }
}

pub const HELP: &str = "pill · consola local\n\nUso: pill [--project RUTA] [--config ARCHIVO] [--model NOMBRE]\n\n  --project RUTA     Proyecto activo; por defecto, la carpeta actual.\n  --config ARCHIVO   Preferencias JSON; las rutas relativas parten del proyecto.\n  --model NOMBRE     Elegir explícitamente un modelo ya instalado en Ollama.\n  -h, --help         Mostrar esta ayuda.\n  -V, --version      Mostrar la versión.\n\nEscribe / dentro de pill para ver los comandos disponibles.\nCtrl+Enter envía; Enter añade línea; Esc cancela; Ctrl+C sale.\nLa interfaz se inicia solo desde una consola interactiva.\n";

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn paths_with_spaces_and_unicode_are_single_arguments() {
        assert_eq!(
            Options::parse(args(&[
                "--project",
                "D:\\Mi código",
                "--config",
                "preferencias.json"
            ]))
            .unwrap(),
            Options::Launch {
                project: Some(PathBuf::from("D:\\Mi código")),
                config: Some(PathBuf::from("preferencias.json")),
                model: None,
            }
        );
    }

    #[test]
    fn unknown_missing_and_duplicate_options_fail() {
        for values in [
            &["--project"][..],
            &["--project", "--config", "x"],
            &["--project", "x", "--project", "y"],
            &["--model"],
            &["--model", "x", "--model", "y"],
        ] {
            assert!(Options::parse(args(values)).is_err());
        }
    }
}
