use super::{
    model::{App, Key},
    render,
};
use crate::{
    config::Config,
    files::Files,
    session::{Action, Event, Worker},
};
use std::{
    io::{self, Write},
    mem::zeroed,
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{Console::*, Threading::WaitForSingleObject},
    UI::Input::KeyboardAndMouse::*,
};

struct Console {
    input: HANDLE,
    output: HANDLE,
    input_mode: u32,
    output_mode: u32,
    cursor: CONSOLE_CURSOR_INFO,
    attributes: u16,
    active: bool,
}

impl Console {
    fn open() -> io::Result<Self> {
        // Los manejadores pertenecen al proceso; nunca se cierran con CloseHandle.
        unsafe {
            let input = GetStdHandle(STD_INPUT_HANDLE);
            let output = GetStdHandle(STD_OUTPUT_HANDLE);
            if input.is_null()
                || output.is_null()
                || input == INVALID_HANDLE_VALUE
                || output == INVALID_HANDLE_VALUE
            {
                return Err(io::Error::last_os_error());
            }
            let (mut input_mode, mut output_mode) = (0, 0);
            if GetConsoleMode(input, &mut input_mode) == 0
                || GetConsoleMode(output, &mut output_mode) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let mut cursor = zeroed::<CONSOLE_CURSOR_INFO>();
            let mut info = zeroed::<CONSOLE_SCREEN_BUFFER_INFO>();
            if GetConsoleCursorInfo(output, &mut cursor) == 0
                || GetConsoleScreenBufferInfo(output, &mut info) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let mut console = Self {
                input,
                output,
                input_mode,
                output_mode,
                cursor,
                attributes: info.wAttributes,
                active: true,
            };
            let raw = (input_mode | ENABLE_EXTENDED_FLAGS | ENABLE_WINDOW_INPUT)
                & !(ENABLE_ECHO_INPUT
                    | ENABLE_LINE_INPUT
                    | ENABLE_PROCESSED_INPUT
                    | ENABLE_QUICK_EDIT_MODE
                    | ENABLE_VIRTUAL_TERMINAL_INPUT);
            if SetConsoleMode(input, raw) == 0
                || SetConsoleMode(
                    output,
                    output_mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
                ) == 0
            {
                let error = io::Error::last_os_error();
                console.restore()?;
                return Err(error);
            }
            write!(io::stdout(), "\x1b[?1049h\x1b[?25l")?;
            io::stdout().flush()?;
            Ok(console)
        }
    }

    fn restore(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        let output = (|| {
            write!(io::stdout(), "\x1b[0m\x1b[?25h\x1b[?1049l")?;
            io::stdout().flush()
        })();
        let input_restored = unsafe { SetConsoleMode(self.input, self.input_mode) };
        let output_restored = unsafe { SetConsoleMode(self.output, self.output_mode) };
        let cursor_restored = unsafe { SetConsoleCursorInfo(self.output, &self.cursor) };
        let attributes_restored = unsafe { SetConsoleTextAttribute(self.output, self.attributes) };
        if input_restored == 0
            || output_restored == 0
            || cursor_restored == 0
            || attributes_restored == 0
        {
            return Err(io::Error::last_os_error());
        }
        output
    }

    fn size(&self) -> io::Result<(usize, usize)> {
        let mut info = unsafe { zeroed::<CONSOLE_SCREEN_BUFFER_INFO>() };
        if unsafe { GetConsoleScreenBufferInfo(self.output, &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((
            (info.srWindow.Right - info.srWindow.Left + 1) as usize,
            (info.srWindow.Bottom - info.srWindow.Top + 1) as usize,
        ))
    }
}
impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

pub fn run(project: &Path, config: &Config, source: Option<&Path>) -> io::Result<()> {
    let trust = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| io::Error::other("LOCALAPPDATA no está definido para concesiones privadas"))?
        .join("PILLCLI/trust.json");
    let mut worker = Worker::start_project(project, config, source, &trust)?;
    let mut console = Console::open()?;
    let result = event_loop(
        &console,
        project,
        config,
        source,
        &mut worker,
        config.ollama.model.as_deref(),
    );
    let restore = console.restore();
    result.and(restore)
}

fn event_loop(
    console: &Console,
    project: &Path,
    config: &Config,
    source: Option<&Path>,
    worker: &mut Worker,
    initial_model: Option<&str>,
) -> io::Result<()> {
    let files = Files::new(project, config.files.clone()).map_err(io::Error::other)?;
    let mut app = App::new(config.ui.max_input_chars, config.ui.max_history_entries);
    app.load_local_history(&files);
    app.context_limit = Some(config.ollama.num_ctx);
    app.begin(
        initial_model.map_or(Action::Discover, |m| Action::Select(m.into())),
        "Conectando con Ollama…",
    );
    let mut dirty = true;
    let mut high_surrogate = None;
    let mut dimensions = console.size()?;
    loop {
        if let Some(action) = app.take_action()
            && let Err(error) = worker.submit(action)
        {
            app.event(Event::Failed(error));
        }
        while let Some(event) = worker.next_event() {
            let was_busy = app.busy;
            app.event(event);
            if was_busy && !app.busy {
                app.save_local_history(&files);
            }
            dirty = true;
        }
        if dirty {
            let frame = render::frame(
                &mut app,
                project,
                &config.ui,
                source,
                dimensions.0,
                dimensions.1,
            );
            write!(
                io::stdout(),
                "\x1b[?25l{}\x1b[{};1H{}\x1b[?25h",
                frame.text,
                frame.cursor_row,
                frame.cursor_text
            )?;
            io::stdout().flush()?;
            dirty = false;
        }
        match unsafe { WaitForSingleObject(console.input, config.ui.poll_ms) } {
            WAIT_FAILED => return Err(io::Error::last_os_error()),
            WAIT_TIMEOUT => {
                let size = console.size()?;
                if size != dimensions {
                    dimensions = size;
                    dirty = true;
                }
                if app.busy {
                    dirty = true;
                }
                continue;
            }
            WAIT_OBJECT_0 => {}
            value => {
                return Err(io::Error::other(format!(
                    "Espera de consola inesperada: {value}"
                )));
            }
        }
        let mut record = unsafe { zeroed::<INPUT_RECORD>() };
        let mut count = 0;
        if unsafe { ReadConsoleInputW(console.input, &mut record, 1, &mut count) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if count == 0 {
            continue;
        }
        if record.EventType == WINDOW_BUFFER_SIZE_EVENT as u16 {
            dimensions = console.size()?;
            dirty = true;
        }
        if record.EventType != KEY_EVENT as u16 {
            continue;
        }
        let event = unsafe { record.Event.KeyEvent };
        if event.bKeyDown == 0 {
            continue;
        }
        let unit = unsafe { event.uChar.UnicodeChar };
        let ctrl = event.dwControlKeyState & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0;
        let key = match event.wVirtualKeyCode {
            VK_BACK => Some(Key::Backspace),
            VK_DELETE => Some(Key::Delete),
            VK_LEFT if ctrl => Some(Key::WordLeft),
            VK_RIGHT if ctrl => Some(Key::WordRight),
            VK_LEFT => Some(Key::Left),
            VK_RIGHT => Some(Key::Right),
            VK_HOME => Some(Key::Home),
            VK_END => Some(Key::End),
            VK_UP => Some(Key::Up),
            VK_DOWN => Some(Key::Down),
            VK_PRIOR => Some(Key::PageUp),
            VK_NEXT => Some(Key::PageDown),
            VK_R if ctrl => Some(Key::ToggleReasoning),
            VK_RETURN
                if event.dwControlKeyState & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0 =>
            {
                Some(Key::Submit)
            }
            VK_RETURN => Some(Key::Enter),
            VK_ESCAPE => Some(Key::Escape),
            VK_TAB => Some(Key::Tab),
            _ if unit == 3 => Some(Key::Interrupt),
            _ => {
                if (0xD800..=0xDBFF).contains(&unit) {
                    high_surrogate = Some(unit);
                    continue;
                }
                let value = if (0xDC00..=0xDFFF).contains(&unit) {
                    high_surrogate.take().and_then(|high| {
                        char::from_u32(
                            0x10000 + ((u32::from(high) - 0xD800) << 10) + u32::from(unit) - 0xDC00,
                        )
                    })
                } else {
                    high_surrogate = None;
                    char::from_u32(u32::from(unit))
                };
                value
                    .filter(|c| !c.is_control())
                    .map(|c| Key::Text(c, event.wRepeatCount))
            }
        };
        if let Some(key) = key {
            app.view_rows = dimensions.1;
            if app.key(key) {
                app.save_local_history(&files);
                return Ok(());
            }
            dirty = true;
        }
    }
}
