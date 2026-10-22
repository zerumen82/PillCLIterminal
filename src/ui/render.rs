use super::model::App;
use crate::config::{LogoColor, UiConfig};
use std::path::Path;

/// Una celda conservadora para texto no ASCII; no divide caracteres UTF-8.
fn width(c: char) -> usize {
    if c.is_ascii() { 1 } else { 2 }
}

fn shown_status(app: &App) -> String {
    match app.busy_at {
        Some(start) if app.busy => format!("{} · {} s", app.status, start.elapsed().as_secs()),
        _ => app.status.clone(),
    }
}

pub fn clean(text: &str, columns: usize) -> String {
    let mut used = 0;
    text.chars()
        .filter(|c| !c.is_control())
        .take_while(|c| {
            used += width(*c);
            used <= columns
        })
        .collect()
}

pub struct Frame {
    pub text: String,
    pub cursor_row: usize,
    pub cursor_text: String,
}

pub fn frame(
    app: &mut App,
    project: &Path,
    config: &UiConfig,
    source: Option<&Path>,
    columns: usize,
    rows: usize,
) -> Frame {
    let columns = columns.saturating_sub(1);
    let mut lines = Vec::new();
    let color = match config.logo_color {
        LogoColor::Green => 32,
        LogoColor::Cyan => 36,
        LogoColor::White => 37,
    };
    let logo = if config.ascii_logo {
        include_str!("../../assets/capsule-ascii.txt")
    } else {
        include_str!("../../assets/capsule.txt")
    };
    for (index, line) in logo.lines().enumerate() {
        let label = if index == 1 {
            format!("{line}  pill")
        } else {
            line.to_owned()
        };
        lines.push(format!("\x1b[{color}m{}\x1b[0m", clean(&label, columns)));
    }
    lines.push(clean(&format!("Proyecto: {}", project.display()), columns));
    lines.push(clean(
        &format!(
            "Configuración: {}",
            source
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "valores incluidos".into())
        ),
        columns,
    ));
    lines.push(clean(&format!("Modelo: {}", app.model_label), columns));
    if !app.digest.is_empty() {
        lines.push(clean(&format!("Digest: {}", app.digest), columns));
    }
    lines.push(clean(&shown_status(app), columns));
    let mode = match (app.task_mode, app.task_step.as_ref()) {
        (true, Some((role, step))) => format!("Modo: Tarea · {role} · paso {step}"),
        (true, None) => "Modo: Tarea".into(),
        (false, _) => "Modo: Chat".into(),
    };
    lines.push(clean(&mode, columns));
    lines.push(clean(&context_line(app), columns));
    lines.push(clean(&memory_line(app), columns));
    if app.partial_effects {
        let banner = if app.journal_hint.is_empty() {
            "Efectos parciales: las escrituras siguen bloqueadas. Revisa el diario.".to_owned()
        } else {
            format!(
                "Efectos parciales: las escrituras siguen bloqueadas. Revisa {}.",
                app.journal_hint
            )
        };
        lines.push(clean(&banner, columns));
    }
    let commands = app.matches();
    let choices = if app.menu_open() {
        Vec::new()
    } else {
        app.choice_rows()
    };
    // Reserva el borrador, el menú, el pie y una fila de margen.
    let menu_rows = if app.menu_open() {
        commands.len().max(1)
    } else {
        choices.len()
    };
    let input = input_view(app, columns);
    let available = rows.saturating_sub(lines.len() + menu_rows + input.lines.len() + 2);
    let mut history = Vec::new();
    for entry in app
        .history
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(app.response.as_str()))
    {
        for logical in entry.lines() {
            let mut line = String::new();
            let mut used = 0;
            for c in logical.chars().filter(|c| !c.is_control()) {
                if used + width(c) > columns && !line.is_empty() {
                    history.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push(c);
                used += width(c);
            }
            history.push(clean(&line, columns));
        }
    }
    // Una fila propia por aviso: el pie ya ocupa el ancho y, si se le añade, se recorta.
    let (scroll, available, hidden_above, hidden_below) =
        fit_history(history.len(), app.scroll, available);
    app.scroll = scroll;
    lines.extend(
        history
            .into_iter()
            .rev()
            .skip(scroll)
            .take(available)
            .collect::<Vec<_>>()
            .into_iter()
            .rev(),
    );
    let cursor_row = lines.len() + 1 + input.cursor_offset;
    let cursor_text = input.cursor_text;
    lines.extend(input.lines);
    if app.menu_open() {
        if commands.is_empty() {
            lines.push(clean("Sin coincidencias", columns));
        }
        for (index, command) in commands.iter().enumerate() {
            let marker = if index == app.selected { ">" } else { " " };
            lines.push(clean(
                &format!("{marker} {}  {}", command.name, command.description),
                columns,
            ));
        }
    } else {
        for row in &choices {
            lines.push(clean(row, columns));
        }
    }
    if hidden_below {
        lines.push(clean("↓ hay texto por debajo", columns));
    }
    if hidden_above {
        lines.push(clean("↑ hay texto por encima", columns));
    }
    lines.push(clean(
        "/ comandos · Ctrl+Enter enviar · Enter línea · Esc cancelar · Ctrl+C salir",
        columns,
    ));
    if rows < 10 || columns < 20 {
        return Frame {
            text: format!(
                "\x1b[H\x1b[2J{}",
                clean("Amplía la consola. Ctrl+C para salir.", columns)
            ),
            cursor_row: 1,
            cursor_text: String::new(),
        };
    }
    lines.truncate(rows.saturating_sub(1));
    Frame {
        text: format!("\x1b[H\x1b[2J{}", lines.join("\r\n")),
        cursor_row,
        cursor_text,
    }
}

fn context_line(app: &App) -> String {
    match (app.context_used, app.context_limit) {
        (Some(used), Some(limit)) => {
            let mut line = format!("Reserva: {used}/{limit}");
            if u128::try_from(used).unwrap_or(u128::MAX) * 100 >= u128::from(limit) * 80 {
                line.push_str(" · cerca del límite");
            }
            line
        }
        (None, Some(limit)) => format!("Reserva: —/{limit}"),
        _ => "Reserva: sin medir".into(),
    }
}

/// Reparte el alto visible. `scroll` oculta las líneas nuevas; no puede pasar del inicio.
fn fit_history(len: usize, mut scroll: usize, base: usize) -> (usize, usize, bool, bool) {
    let mut available = base;
    let mut above = false;
    let mut below = false;
    for _ in 0..4 {
        let mut room = base;
        below = scroll > 0;
        if below {
            room = room.saturating_sub(1);
        }
        above = len > scroll.saturating_add(room);
        if above {
            room = room.saturating_sub(1);
            above = len > scroll.saturating_add(room);
        }
        let next = scroll.min(len.saturating_sub(room.max(1)));
        let changed = next != scroll || room != available;
        scroll = next;
        available = room;
        if !changed {
            break;
        }
    }
    if scroll == 0 && below {
        below = false;
        let mut room = base;
        above = len > room;
        if above {
            room = room.saturating_sub(1);
            above = len > room;
        }
        available = room;
    }
    (scroll, available, above, below)
}

fn memory_line(app: &App) -> String {
    let names = if app.session_models.is_empty() {
        "ninguna".to_owned()
    } else {
        app.session_models.join(", ")
    };
    let exclusive = if app.exclusive {
        "exclusividad concedida"
    } else {
        "exclusividad no concedida"
    };
    format!("Memoria de esta sesión: {names} · {exclusive}")
}

struct InputView {
    lines: Vec<String>,
    cursor_offset: usize,
    cursor_text: String,
}

fn input_view(app: &App, columns: usize) -> InputView {
    let cursor = app.cursor.min(app.input.len());
    let parts: Vec<&str> = app.input.split('\n').collect();
    let mut offset = 0usize;
    let mut cursor_line = parts.len().saturating_sub(1);
    let mut cursor_col = 0usize;
    for (index, line) in parts.iter().enumerate() {
        let end = offset + line.len();
        if cursor <= end {
            cursor_line = index;
            cursor_col = cursor - offset;
            break;
        }
        offset = end + 1;
    }
    let start = cursor_line.saturating_sub(3);
    let end = (start + 4).min(parts.len());
    let mut lines = Vec::new();
    let mut cursor_text = String::new();
    let mut cursor_offset = 0usize;
    for (index, line) in parts.iter().enumerate().skip(start).take(end - start) {
        let marker = if index == 0 { "> " } else { "  " };
        let line = *line;
        let budget = columns.saturating_sub(marker.len());
        if index == cursor_line {
            cursor_offset = lines.len();
            let before = &line[..cursor_col];
            let mut left: Vec<char> = before.chars().rev().take(budget).collect();
            while left.iter().copied().map(width).sum::<usize>() > budget {
                left.pop();
            }
            left.reverse();
            let left: String = left.into_iter().collect();
            cursor_text = clean(&format!("{marker}{left}"), columns);
            lines.push(clean(
                &format!("{marker}{left}{}", &line[cursor_col..]),
                columns,
            ));
        } else {
            lines.push(clean(&format!("{marker}{line}"), columns));
        }
    }
    if lines.is_empty() {
        cursor_text = clean("> ", columns);
        lines.push(cursor_text.clone());
    }
    InputView {
        lines,
        cursor_offset,
        cursor_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_characters_cannot_inject_terminal_sequences() {
        assert_eq!(clean("a\x1b[31m\nZ", 40), "a[31mZ");
        assert_eq!(clean("á🟢x", 4), "á🟢");
    }

    #[test]
    fn multiline_draft_keeps_the_cursor_on_its_own_line() {
        let mut app = App::new(128, 64);
        app.context_limit = Some(4096);
        app.input = "uno\ndos".into();
        app.cursor = app.input.len();
        for index in 0..30 {
            app.message(format!("linea {index}"));
        }
        let config = UiConfig {
            ascii_logo: true,
            logo_color: LogoColor::Green,
            poll_ms: 100,
            max_input_chars: 128,
            max_history_entries: 64,
        };
        let frame = frame(&mut app, Path::new("proyecto"), &config, None, 80, 40);
        assert!(frame.text.contains("Modo: Chat"));
        assert!(frame.text.contains("Reserva: —/4096"));
        assert!(frame.text.contains("Memoria de esta sesión: ninguna"));
        assert!(frame.text.contains("hay texto por encima"));
        assert!(frame.cursor_text.contains("dos"), "{}", frame.cursor_text);
        assert!(!frame.cursor_text.contains("uno"), "{}", frame.cursor_text);
        let rows: Vec<&str> = frame.text.split("\r\n").collect();
        let first = rows
            .iter()
            .position(|line| line.contains("> uno"))
            .expect("primera línea del borrador");
        assert_eq!(frame.cursor_row, first + 2);
    }

    #[test]
    fn reserve_task_journal_and_scroll_stay_on_screen() {
        let mut app = App::new(128, 64);
        app.context_used = Some(3300);
        app.context_limit = Some(4096);
        app.task_mode = true;
        app.task_step = Some(("tester".into(), 3));
        app.partial_effects = true;
        app.journal_hint = ".pillcli/sessions/1.jsonl".into();
        app.message("linea vieja");
        app.message("linea nueva");
        app.scroll = 10_000;
        let config = UiConfig {
            ascii_logo: true,
            logo_color: LogoColor::Green,
            poll_ms: 100,
            max_input_chars: 128,
            max_history_entries: 64,
        };
        let wide = frame(&mut app, Path::new("proyecto"), &config, None, 80, 40);
        assert!(wide.text.contains("Reserva: 3300/4096 · cerca del límite"));
        assert!(wide.text.contains("Modo: Tarea · tester · paso 3"));
        assert!(wide.text.contains(".pillcli"));
        assert!(wide.text.contains("linea vieja"));
        assert_eq!(app.scroll, 0);
        for index in 0..30 {
            app.message(format!("bloque {index}"));
        }
        app.scroll = 2;
        let narrow = frame(&mut app, Path::new("proyecto"), &config, None, 80, 24);
        assert!(app.scroll <= 2);
        assert!(narrow.text.contains("hay texto por debajo"));
    }

    #[test]
    fn busy_status_shows_elapsed_seconds_without_replacing_the_activity() {
        let mut app = App::new(128, 16);
        app.busy = true;
        app.status = "razonando · 12 caracteres".into();
        app.busy_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(4));
        let config = UiConfig {
            ascii_logo: true,
            logo_color: LogoColor::Green,
            poll_ms: 100,
            max_input_chars: 128,
            max_history_entries: 64,
        };
        let frame = frame(&mut app, Path::new("proyecto"), &config, None, 80, 40);
        assert!(frame.text.contains("razonando · 12 caracteres · 4 s"));
        assert_eq!(app.status, "razonando · 12 caracteres");
    }
}
