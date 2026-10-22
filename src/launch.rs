use std::io::{self, IsTerminal};

/// Exige una terminal interactiva y una consola Windows con varios procesos.
///
/// Una consola propia con un solo proceso, como la observada en el sondeo de
/// doble clic, se rechaza. La cantidad de procesos no identifica al invocador
/// ni demuestra la antigüedad de la consola.
pub fn require_interactive_console() -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "pill se inicia desde una consola interactiva, sin redirigir entrada ni salida.",
        ));
    }

    #[cfg(windows)]
    {
        let mut process_ids = [0_u32; 1];
        // SAFETY: el buffer es válido y su longitud coincide con el argumento.
        // Windows devuelve la cantidad total de procesos asociados a la consola,
        // aunque esa cantidad sea mayor que la capacidad del buffer suministrado.
        let count = unsafe {
            windows_sys::Win32::System::Console::GetConsoleProcessList(
                process_ids.as_mut_ptr(),
                process_ids.len() as u32,
            )
        };
        if count == 0 {
            return Err(io::Error::last_os_error());
        }
        if count == 1 {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Abre una consola existente y escribe pill; no se inicia mediante doble clic.",
            ));
        }
    }

    Ok(())
}
