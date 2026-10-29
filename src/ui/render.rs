use super::model::App;
use crate::config::{LogoColor, UiConfig};
use std::path::Path;

/// Una celda conservadora para texto no ASCII; no divide caracteres UTF-8.
fn width(c: char) -> usize {
    super::text::scalar_width(c)
}

fn shown_status(app: &App) -> String {
    match app.busy_at {
        Some(start) if app.busy => format!("{} · {} s", app.status, start.elapsed().as_secs()),
        _ => app.status.clone(),
    }
}

pub fn clean(text: &str, columns: usize) -> String {
    let safe: String = text.chars().filter(|c| !c.is_control()).collect();
    let mut shown = String::new();
    let mut used = 0;
    for (cluster, width) in super::text::clusters(&safe) {
        if used + width > columns {
            break;
        }
        shown.push_str(cluster);
        used += width;
    }
    shown
}

pub(super) fn wrap_text(text: &str, columns: usize) -> Vec<String> {
    if text.is_empty() || columns == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for line in text.split('\n') {
        let safe: String = line.chars().filter(|c| !c.is_control()).collect();
        let mut row = String::new();
        let mut used = 0;
        for (cluster, width) in super::text::clusters(&safe) {
            if used + width > columns && !row.is_empty() {
                lines.push(std::mem::take(&mut row));
                used = 0;
            }
            if width <= columns {
                row.push_str(cluster);
                used += width;
            }
        }
        lines.push(row);
    }
    lines
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
    if config.compact_header {
        let logo = if config.ascii_logo {
            include_str!("../../assets/capsule-compact-ascii.txt")
        } else {
            include_str!("../../assets/capsule-compact.txt")
        };
        let project_name = project
            .file_name()
            .unwrap_or(project.as_os_str())
            .to_string_lossy();
        let label=clean(&format!("{}  pill · {project_name}",logo.trim()),columns);
        lines.push(if config.color{format!("\x1b[{color}m{label}\x1b[0m")}else{label});
        lines.push(clean(
            &format!(
                "{} · {} · {}",
                app.mode_label(),
                app.model_label,
                context_line(app)
            ),
            columns,
        ));
        lines.push(clean(&shown_status(app), columns));
    } else {
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
            let label=clean(&label,columns);
            lines.push(if config.color{format!("\x1b[{color}m{label}\x1b[0m")}else{label});
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
        let mode = format!("Modo: {}", app.mode_label());
        lines.push(clean(&mode, columns));
        lines.push(clean(&context_line(app), columns));
        lines.push(clean(&memory_line(app), columns));
    }
    for warning in [&app.model_storage_warning, &app.console_storage_warning]
        .into_iter()
        .flatten()
    {
        lines.push(clean(warning, columns));
    }
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
    if let Some(heading) = app.recovery_heading() {
        lines.push(clean(heading, columns));
    }
    if let Some(detail) = &app.detail_view {
        lines.push(clean(&detail.title, columns));
    }
    let commands = app.matches();
    let menu_open = app.menu_open();
    let has_selection = menu_open || !app.choice_rows(1).is_empty();
    // El pie y el cursor se reservan antes de elegir cuántas opciones caben.
    // La última fila queda libre para que conhost no desplace la pantalla.
    let height = rows.saturating_sub(1);
    let minimum = lines.len() + 2 + usize::from(has_selection);
    if columns < 20 || height < minimum {
        return small_frame(columns);
    }
    let body_rows = height - lines.len() - 1;
    let input_rows = body_rows
        .saturating_sub(usize::from(has_selection))
        .min(config.max_input_rows);
    let input = input_view(app, columns, input_rows);
    let option_rows = body_rows - input.lines.len();
    let choice_budget = if app.detail_view.is_some() {
        option_rows.min(3)
    } else {
        option_rows
    };
    let mut selection = Vec::new();
    if menu_open {
        if commands.is_empty() {
            selection.push("Sin coincidencias".to_owned());
        } else {
            let window = choice_budget.min(commands.len());
            let pick = app.selected.min(commands.len() - 1);
            let start = pick.saturating_sub(window / 2).min(commands.len() - window);
            for (index, command) in commands.iter().enumerate().skip(start).take(window) {
                let marker = if index == pick { ">" } else { " " };
                selection.push(format!(
                    "{marker} {}  {}",
                    command.name, command.description
                ));
            }
        }
    } else {
        selection = app.choice_rows(choice_budget);
    }
    let available = option_rows - selection.len();
    let detail_open = app.detail_view.is_some();
    // Arc comparte las cadenas envueltas; solo se copian las filas del frame.
    let cached = (!detail_open).then(|| app.wrapped_history(columns));
    let history: &[String] = cached.as_deref().map_or(&[], Vec::as_slice);
    let (updates,tones) = if let Some(detail) = &app.detail_view {
        detail_rows(detail,columns)
    } else {
        let mut rows=wrap_text(&app.response,columns);
        if !app.response.is_empty(){rows.insert(0,"Respuesta (en curso):".into());rows.insert(0,String::new());}
        (rows,Vec::new())
    };
    let history_len = history.len() + updates.len();
    // Una fila propia por aviso: el pie ya ocupa el ancho y, si se le añade, se recorta.
    let requested = app.detail_view.as_ref().map_or(app.scroll, |d| d.scroll);
    let (scroll, available, hidden_above, hidden_below) = if detail_open {
        fit_detail(history_len, requested, available)
    } else {
        fit_history(history_len, requested, available)
    };
    if let Some(detail) = &mut app.detail_view {
        detail.scroll = scroll;
    } else {
        app.scroll = scroll;
    }
    app.history_rows = available.max(1);
    let (start, end) = if detail_open {
        (scroll, scroll.saturating_add(available).min(history_len))
    } else {
        let end = history_len.saturating_sub(scroll);
        (end.saturating_sub(available), end)
    };
    let visible_start=lines.len();
    append_visible(&mut lines, history, &updates, start, end);
    if detail_open && config.color {
        for (line,tone) in lines[visible_start..].iter_mut().zip(tones[start..end].iter()) {
            let color=match tone {Some(super::diff::DiffTone::Added)=>Some(32),Some(super::diff::DiffTone::Removed)=>Some(31),Some(super::diff::DiffTone::Hunk)=>Some(36),_=>None};
            if let Some(color)=color {*line=format!("\x1b[{color}m{line}\x1b[0m");}
        }
    }
    let cursor_row = lines.len() + 1 + input.cursor_offset;
    let cursor_text = input.cursor_text;
    lines.extend(input.lines);
    for row in selection {
        lines.push(clean(&row, columns));
    }
    if hidden_below {
        lines.push(clean("↓ hay texto por debajo", columns));
    }
    if hidden_above {
        lines.push(clean("↑ hay texto por encima", columns));
    }
    lines.push(clean(app.footer(), columns));
    Frame {
        text: format!("\x1b[H\x1b[2J{}", lines.join("\r\n")),
        cursor_row,
        cursor_text,
    }
}

fn detail_rows(view:&super::model::DetailView,columns:usize)->(Vec<String>,Vec<Option<super::diff::DiffTone>>) {
    use super::diff::DiffTone;
    let mut rows=Vec::new();let mut tones=Vec::new();
    if view.text.is_empty()||columns==0{return(rows,tones);}
    for (index,line) in view.text.split('\n').enumerate(){
        let tone=view.highlights.get(&index).copied();
        let marker=match tone{Some(DiffTone::Added)=>Some("+"),Some(DiffTone::Removed)=>Some("-"),Some(DiffTone::Context)=>Some(" "),_=>None};
        let body=marker.and_then(|marker|line.strip_prefix(marker)).unwrap_or(line);
        let available=columns.saturating_sub(if marker.is_some(){2}else{0});
        let wrapped=if body.is_empty(){vec![String::new()]}else{wrap_text(body,available)};
        for row in wrapped {rows.push(if let Some(marker)=marker{format!("{marker} {row}")}else{row});tones.push(tone);}
    }
    (rows,tones)
}

/// Selecciona la ventana sobre dos slices, sin recorrer ni copiar su prefijo.
fn append_visible(
    output: &mut Vec<String>,
    history: &[String],
    updates: &[String],
    start: usize,
    end: usize,
) {
    let saved_start = start.min(history.len());
    let saved_end = end.min(history.len());
    output.extend(history[saved_start..saved_end].iter().cloned());
    let new_start = start.saturating_sub(history.len()).min(updates.len());
    let new_end = end.saturating_sub(history.len()).min(updates.len());
    output.extend(updates[new_start..new_end].iter().cloned());
}

fn fit_detail(len: usize, requested: usize, base: usize) -> (usize, usize, bool, bool) {
    let start = requested.min(len.saturating_sub(1));
    let above = start > 0 && base > 1;
    let room = base.saturating_sub(usize::from(above));
    let below = room > 1 && len > start.saturating_add(room);
    (start, room.saturating_sub(usize::from(below)), above, below)
}

fn small_frame(columns: usize) -> Frame {
    Frame {
        text: format!(
            "\x1b[H\x1b[2J{}",
            clean("Amplía la consola. Ctrl+C para salir.", columns)
        ),
        cursor_row: 1,
        cursor_text: String::new(),
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
fn fit_history(len: usize, scroll: usize, base: usize) -> (usize, usize, bool, bool) {
    if base == 0 {
        return (scroll.min(len.saturating_sub(1)), 0, false, false);
    }
    if base == 1 {
        // Con una única fila prima el contenido sobre los avisos de scroll.
        return (scroll.min(len.saturating_sub(1)), 1, false, false);
    }
    let mut scroll = scroll.min(len.saturating_sub(1));
    loop {
        let below = scroll > 0;
        let room = base - usize::from(below);
        // Los avisos jamás consumen la última fila de contenido disponible.
        let above = room > 1 && len > scroll.saturating_add(room);
        let room = room - usize::from(above);
        let next = scroll.min(len.saturating_sub(room));
        if next == scroll {
            return (scroll, room, above, below);
        }
        scroll = next;
    }
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

fn input_view(app: &App, columns: usize, max_rows: usize) -> InputView {
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
    let window = max_rows.max(1).min(parts.len());
    let start = cursor_line.saturating_sub(window - 1);
    let end = (start + window).min(parts.len());
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
    use crate::{
        session::{Event, ListedModel},
        tools::mcp::Offer,
        ui::model::Key,
    };
    #[test]
    fn control_characters_cannot_inject_terminal_sequences() {
        assert_eq!(clean("a\x1b[31m\nZ", 40), "a[31mZ");
        assert_eq!(clean("á🟢x", 4), "á🟢x");
        assert_eq!(clean("a\u{301}b", 1), "a\u{301}");
        assert_eq!(clean("👨‍👩‍👧‍👦x", 2), "👨‍👩‍👧‍👦");
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
            ..test_config_defaults()
        };
        let frame = frame(&mut app, Path::new("proyecto"), &config, None, 80, 40);
        assert!(frame.text.contains("Modo: Editar"));
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
            ..test_config_defaults()
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
            ..test_config_defaults()
        };
        let frame = frame(&mut app, Path::new("proyecto"), &config, None, 80, 40);
        assert!(frame.text.contains("razonando · 12 caracteres · 4 s"));
        assert_eq!(app.status, "razonando · 12 caracteres");
    }

    fn test_config_defaults() -> UiConfig {
        let mut config: crate::config::Config =
            serde_json::from_str(include_str!("../../config/defaults.json")).unwrap();
        config.ui.compact_header = false;
        config.ui
    }

    fn test_config() -> UiConfig {
        UiConfig {
            ascii_logo: true,
            logo_color: LogoColor::Green,
            poll_ms: 100,
            max_input_chars: 4096,
            max_history_entries: 128,
            ..test_config_defaults()
        }
    }

    fn assert_frame_bounds(frame: &Frame, rows: usize) {
        let lines: Vec<_> = frame.text.split("\r\n").collect();
        assert!(lines.len() < rows, "{} líneas en {rows} filas", lines.len());
        assert!(
            lines.last().unwrap().starts_with("/ comandos"),
            "{}",
            frame.text
        );
        assert!(
            (1..=lines.len()).contains(&frame.cursor_row),
            "cursor={} líneas={}",
            frame.cursor_row,
            lines.len()
        );
        assert!(
            lines[frame.cursor_row - 1].starts_with(&frame.cursor_text),
            "cursor fuera del borrador: {}",
            frame.text
        );
    }

    #[test]
    fn resized_model_lists_keep_last_selection_and_footer_visible() {
        for count in [8, 20] {
            for selected in [0, count / 2, count - 1] {
                let mut app = App::new(4096, 128);
                app.digest = "digest que reserva una fila real".into();
                app.event(Event::Models(
                    (0..count)
                        .map(|index| ListedModel {
                            name: format!("modelo-{index:02}"),
                            size: 1024,
                            loaded: Some(false),
                        })
                        .collect(),
                ));
                for _ in 0..selected {
                    app.key(Key::Down);
                }
                let visible = frame(
                    &mut app,
                    Path::new("proyecto"),
                    &test_config(),
                    None,
                    73,
                    21,
                );
                assert_frame_bounds(&visible, 21);
                assert!(
                    visible
                        .text
                        .contains(&format!("> {} · modelo-{selected:02}", selected + 1)),
                    "{}",
                    visible.text
                );
                assert!(visible.text.contains("Elige un número"));
            }
        }
    }

    #[test]
    fn long_drafts_and_permission_lists_keep_cursor_and_marked_row_visible() {
        for count in [8, 20] {
            for cursor_line in [0, 19, 39] {
                let mut app = App::new(4096, 128);
                app.digest = "digest".into();
                app.partial_effects = true;
                app.journal_hint = ".pillcli/sessions/prueba.jsonl".into();
                app.event(Event::Permissions(
                    (0..count)
                        .map(|index| Offer {
                            id: format!("command:prueba-{index:02}"),
                            fingerprint: index.to_string(),
                            detail: r#"{"granted":false}"#.into(),
                        })
                        .collect(),
                ));
                for _ in 0..count - 1 {
                    app.key(Key::Down);
                }
                app.input = (0..40)
                    .map(|index| format!("borrador-{index:02}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let current = format!("borrador-{cursor_line:02}");
                app.cursor = app.input.find(&current).unwrap() + current.len();
                let visible = frame(
                    &mut app,
                    Path::new("proyecto"),
                    &test_config(),
                    None,
                    73,
                    21,
                );
                assert_frame_bounds(&visible, 21);
                assert!(
                    visible.cursor_text.ends_with(&current),
                    "{}",
                    visible.cursor_text
                );
                assert!(
                    visible.text.contains(&format!(
                        "> {count} · Ejecutar prueba/comando prueba-{:02}",
                        count - 1
                    )),
                    "{}",
                    visible.text
                );
            }
        }
    }

    #[test]
    fn command_menu_scrolls_to_selected_row_and_small_windows_use_explicit_fallback() {
        let mut app = App::new(4096, 128);
        app.digest = "digest".into();
        app.partial_effects = true;
        app.input = "/".into();
        app.cursor = 1;
        for _ in 0..app.matches().len() - 1 {
            app.key(Key::Down);
        }
        let visible = frame(
            &mut app,
            Path::new("proyecto"),
            &test_config(),
            None,
            73,
            21,
        );
        assert_frame_bounds(&visible, 21);
        assert!(visible.text.contains("> /copy"), "{}", visible.text);
        let small = frame(
            &mut app,
            Path::new("proyecto"),
            &test_config(),
            None,
            73,
            14,
        );
        assert!(small.text.contains("Amplía la consola"));
        assert!(small.text.contains("Ctrl+C"));
        assert_eq!(small.cursor_row, 1);
    }

    #[test]
    fn scroll_indicators_never_displace_all_history_content() {
        for base in 1..=4 {
            for requested in [0, 2, 99] {
                let (scroll, content, above, below) = fit_history(10, requested, base);
                assert!(content > 0);
                assert!(content + usize::from(above) + usize::from(below) <= base);
                assert!(scroll < 10);
            }
        }
    }
    #[test]
    fn compact_details_start_at_top_keep_title_and_page_in_reading_order() {
        let mut app = App::new(4096, 128);
        app.detail_view = Some(crate::ui::model::DetailView {
            title: "Estado de la sesion".into(),
            text: (0..80)
                .map(|i| format!("detail-{i:02}"))
                .collect::<Vec<_>>()
                .join("\n"),
            scroll: 0,
            highlights: Default::default(),
        });
        let config: crate::config::Config =
            serde_json::from_str(include_str!("../../config/defaults.json")).unwrap();
        let first = frame(&mut app, Path::new("project"), &config.ui, None, 73, 21);
        assert!(first.text.contains("Estado de la sesion"));
        assert!(first.text.contains("detail-00"));
        assert!(!first.text.contains("detail-79"));
        app.key(crate::ui::model::Key::PageDown);
        let second = frame(&mut app, Path::new("project"), &config.ui, None, 73, 21);
        assert!(second.text.contains("Estado de la sesion"));
        assert!(!second.text.contains("detail-00"));
        app.key(crate::ui::model::Key::Home);
        let home = frame(&mut app, Path::new("project"), &config.ui, None, 73, 21);
        assert!(home.text.contains("detail-00"));
    }
    // Referencia del camino anterior, solo en pruebas: clonaba la concatenacion
    // completa antes de elegir la ventana. No se usa en produccion.
    fn old_window(
        history: &[String],
        updates: &[String],
        scroll: usize,
        available: usize,
    ) -> Vec<String> {
        let mut joined = history.to_vec();
        joined.extend_from_slice(updates);
        joined
            .into_iter()
            .rev()
            .skip(scroll)
            .take(available)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    #[test]
    fn borrowed_window_matches_previous_selection_for_empty_unicode_response_and_scroll_limits() {
        for count in [0, 1, 2, 31] {
            let history: Vec<_> = (0..count)
                .map(|i| format!("linea-{i}: \u{00e1} \u{4e2d} \u{1f642}"))
                .collect();
            for text in ["", "\n", "respuesta\nsegunda\n"] {
                let updates = wrap_text(text, 17);
                let len = history.len() + updates.len();
                for base in 0..9 {
                    for requested in [0, 1, 7, 99, usize::MAX] {
                        let (scroll, available, _, _) = fit_history(len, requested, base);
                        let end = len.saturating_sub(scroll);
                        let start = end.saturating_sub(available);
                        let mut actual = Vec::new();
                        append_visible(&mut actual, &history, &updates, start, end);
                        assert_eq!(
                            actual,
                            old_window(&history, &updates, scroll, available),
                            "len={len},base={base},scroll={requested}"
                        );
                        let (start, available, _, _) = fit_detail(updates.len(), requested, base);
                        let mut actual = Vec::new();
                        append_visible(
                            &mut actual,
                            &[],
                            &updates,
                            start,
                            start.saturating_add(available).min(updates.len()),
                        );
                        assert_eq!(
                            actual,
                            updates
                                .iter()
                                .skip(start)
                                .take(available)
                                .cloned()
                                .collect::<Vec<_>>()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn rendering_reuses_cached_history_until_content_or_width_changes() {
        let mut app = App::new(4096, 128);
        app.message("historial \u{00e1} ".repeat(200));
        let cfg: crate::config::Config =
            serde_json::from_str(include_str!("../../config/defaults.json")).unwrap();
        let cached = app.wrapped_history(72);
        let first = frame(&mut app, Path::new("project"), &cfg.ui, None, 73, 21);
        app.response = "respuesta\n".into();
        app.scroll = 7;
        frame(&mut app, Path::new("project"), &cfg.ui, None, 73, 21);
        assert!(std::sync::Arc::ptr_eq(&cached, &app.wrapped_history(72)));
        frame(&mut app, Path::new("project"), &cfg.ui, None, 50, 21);
        assert!(!std::sync::Arc::ptr_eq(&cached, &app.wrapped_history(49)));
        let resized = app.wrapped_history(49);
        app.message("nuevo");
        let after = frame(&mut app, Path::new("project"), &cfg.ui, None, 50, 21);
        assert!(!std::sync::Arc::ptr_eq(&resized, &app.wrapped_history(49)));
        assert!(!first.text.is_empty() && !after.text.is_empty());
    }

    #[test]
    #[ignore = "medicion explicita de seleccion sobre cache caliente; no es una inferencia ni una promesa CPU/RAM"]
    fn measure_long_history_selection_copies_only_visible_lines() {
        use std::{hint::black_box, time::Instant};
        let history: Vec<_> = (0..8192)
            .map(|i| format!("line-{i:05}: {}", "a".repeat(64)))
            .collect();
        let updates = Vec::new();
        let len = history.len() + updates.len();
        let total_bytes = history
            .iter()
            .chain(updates.iter())
            .map(String::len)
            .sum::<usize>();
        let mut measurements = Vec::new();
        for sample in 0..5 {
            let mut old_bytes = 0usize;
            let begin = Instant::now();
            for n in 0..128 {
                let (scroll, available, _, _) = fit_history(len, n % 100, 15);
                let result =
                    old_window(black_box(&history), black_box(&updates), scroll, available);
                old_bytes += total_bytes;
                black_box(result);
            }
            let old_ns = begin.elapsed().as_nanos();
            let begin = Instant::now();
            let mut new_bytes = 0usize;
            for n in 0..128 {
                let (scroll, available, _, _) = fit_history(len, n % 100, 15);
                let end = len.saturating_sub(scroll);
                let mut result = Vec::new();
                append_visible(
                    &mut result,
                    black_box(&history),
                    black_box(&updates),
                    end.saturating_sub(available),
                    end,
                );
                new_bytes += result.iter().map(String::len).sum::<usize>();
                black_box(result);
            }
            let new_ns = begin.elapsed().as_nanos();
            assert!(new_bytes < old_bytes);
            measurements.push(serde_json::json!({"sample":sample,"frames":128,"old_elapsed_ns":old_ns,"new_elapsed_ns":new_ns,"old_string_payload_bytes_copied":old_bytes,"new_string_payload_bytes_copied":new_bytes}));
        }
        let report = serde_json::json!({"result":"PASS","debug_assertions":cfg!(debug_assertions),"history_rows":history.len(),"history_and_response_bytes":total_bytes,"samples":measurements,"scope":"Camino de seleccion de cadenas en cache caliente, algoritmo anterior y nuevo con misma ventana. Tiempo de pared, sin inferencia. No mide RAM del proceso completo ni garantiza ahorro CPU universal."});
        if let Some(path) = std::env::var_os("PILL_WINDOW_EVIDENCE") {
            assert!(!Path::new(&path).exists());
            std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
    }
}
