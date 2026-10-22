//! Contratos compartidos del arranque de pill.

pub mod agents;
pub mod cli;
pub mod config;
pub mod files;
pub mod gpu;
pub mod launch;
pub mod providers;
pub mod session;
pub mod tools;
pub mod ui;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Args(String),
    #[error("Configuración: {0}")]
    Config(String),
    #[error("JSON no válido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}
