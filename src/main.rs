use pill::{
    Error,
    cli::{HELP, Options},
    config::Config,
    launch, ui,
};
use std::{env, path::PathBuf, process::ExitCode};

fn execute() -> Result<(), Error> {
    match Options::parse(env::args_os().skip(1))? {
        Options::Help => print!("{HELP}"),
        Options::Version => println!("pill {}", env!("CARGO_PKG_VERSION")),
        Options::Launch {
            project,
            config,
            model,
        } => {
            launch::require_interactive_console()?;
            let project: PathBuf = project.unwrap_or(env::current_dir()?).canonicalize()?;
            if !project.is_dir() {
                return Err(Error::Args("El proyecto debe ser una carpeta".into()));
            }
            let mut loaded = Config::load(&project, config.as_deref())?;
            if let Some(model) = model {
                loaded.config.ollama.model = Some(model);
            }
            ui::run(&project, &loaded)?;
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pill: {error}");
            ExitCode::FAILURE
        }
    }
}
