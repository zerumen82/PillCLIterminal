use std::io::{self, IsTerminal};

#[path = "../../../src/launch.rs"]
mod launch;

fn main() -> io::Result<()> {
    #[cfg(windows)]
    let attached = {
        let mut process_ids = [0_u32; 1];
        // El valor devuelto es la cantidad total, aunque el buffer solo tenga una entrada.
        unsafe {
            windows_sys::Win32::System::Console::GetConsoleProcessList(
                process_ids.as_mut_ptr(),
                process_ids.len() as u32,
            )
        }
    };
    #[cfg(not(windows))]
    let attached = 0;
    let mut report = format!(
        "stdin_terminal={} stdout_terminal={} console_processes={attached}",
        io::stdin().is_terminal(),
        io::stdout().is_terminal()
    );
    let status = match launch::require_interactive_console() {
        Ok(()) => "arranque_interactivo=permitido".to_owned(),
        Err(error) => format!("arranque_interactivo=rechazado: {error}"),
    };
    report.push('\n');
    report.push_str(&status);
    report.push('\n');
    print!("{report}");
    if let Some(path) = std::env::args_os().nth(1) {
        std::fs::write(path, report)?;
    }
    Ok(())
}
