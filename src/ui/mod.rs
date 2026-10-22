pub mod model;
#[cfg(windows)]
mod native;
mod render;

use crate::{Error, config::LoadedConfig};
use std::path::Path;

pub fn run(project: &Path, config: &LoadedConfig) -> Result<(), Error> {
    #[cfg(windows)]
    {
        native::run(project, &config.config, config.source.as_deref())?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (project, config);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Esta interfaz requiere una consola de Windows",
        )
        .into())
    }
}
