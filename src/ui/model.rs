use crate::{
    session::{Action, Event, ListedModel},
    tools::mcp::Offer,
};
use serde_json::{Value, json};
use std::{collections::VecDeque, sync::Arc, time::Instant};

const REASONING_MARK: &str = "Razonamiento abierto";
const REASONING_VIEW_CHARS: usize = 4000;
const LOCAL_HISTORY_MARK: &str = "Historial local de la consola. El modelo solo retoma la conversación si vuelves a elegir el mismo modelo; si eliges otro, empieza una conversación nueva.";
pub(crate) const WELCOME: &str = "Escribe / para abrir el menú. /help muestra la ayuda.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Text(char, u16),
    Backspace,
    Delete,
    Left,
    Right,
    WordLeft,
    WordRight,
    Home,
    End,
    Up,
    Down,
    Enter,
    Submit,
    Escape,
    Tab,
    Interrupt,
    PageUp,
    PageDown,
    ToggleReasoning,
    AltP,
    AltE,
    AltT,
    Copy,
    Gpu,
}

pub struct Command {
    pub name: &'static str,
    pub description: &'static str,
}
pub const COMMANDS: &[Command] = &[
    Command {
        name: "/help",
        description: "Mostrar la ayuda de consola",
    },
    Command {
        name: "/exit",
        description: "Volver a tu consola",
    },
    Command {
        name: "/model",
        description: "Elegir modelo e iniciar otra conversación",
    },
    Command {
        name: "/permissions",
        description: "Ver y conceder comandos, MCP y exclusividad",
    },
    Command {
        name: "/task",
        description: "Activar tarea con agentes y pruebas reales",
    },
    Command {
        name: "/chat",
        description: "Volver al chat con herramientas",
    },
    Command {
        name: "/journal",
        description: "Ver las últimas líneas del diario local",
    },
    Command {
        name: "/liberate",
        description: "Liberar modelos de esta sesión si hay exclusividad concedida",
    },
    Command {
        name: "/status",
        description: "Ver modelo, configuración, recursos y cuotas",
    },
    Command {
        name: "/ask",
        description: "Preguntar con herramientas de solo lectura",
    },
    Command {
        name: "/edit",
        description: "Editar con herramientas y permisos vigentes",
    },
    Command {
        name: "/changes",
        description: "Inspeccionar cambios y resultados observados",
    },
    Command {
        name: "/recover",
        description: "Revisar operaciones pendientes con evidencia",
    },
    Command {
        name: "/undo",
        description: "Restaurar un respaldo con versión comprobada",
    },
    Command {
        name: "/profile",
        description: "Elegir perfil de contexto y generación",
    },
    Command {
        name: "/archive",
        description: "Archivar evidencia revisada con confirmación",
    },
    Command {
        name: "/copy",
        description: "Copiar la última respuesta o el detalle visible",
    },
    Command {
        name: "/gpu",
        description: "Instantánea de VRAM (solo lectura, sin atribución de procesos)",
    },
    Command {
        name: "/init",
        description: "Validar config, PATH y settings OpenCode (solo lectura, sin conceder)",
    },
];

pub(crate) struct DetailView {
    pub title: String,
    pub text: String,
    pub scroll: usize,
    pub(super) highlights: std::collections::BTreeMap<usize, super::diff::DiffTone>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChoiceKind {
    Recovery,
    Changes,
    Undo,
    Profiles,
    Archive,
}

struct StructuredChoices {
    kind: ChoiceKind,
    items: Vec<Value>,
}

pub struct App {
    pub input: String,
    pub cursor: usize,
    pub history: VecDeque<String>,
    pub selected: usize,
    dismissed: bool,
    max_chars: usize,
    max_history: usize,
    pub status: String,
    pub model_label: String,
    pub digest: String,
    pub response: String,
    pub busy: bool,
    pub choosing: bool,
    models: Vec<ListedModel>,
    action: Option<Action>,
    proposals: Vec<Offer>,
    granting: bool,
    pub(crate) task_mode: bool,
    pub scroll: usize,
    sent: VecDeque<String>,
    recalled: Option<usize>,
    draft: String,
    pub(crate) context_used: Option<usize>,
    pub(crate) context_limit: Option<u32>,
    pub(crate) session_models: Vec<String>,
    pub(crate) exclusive: bool,
    pub(crate) partial_effects: bool,
    pub(crate) view_rows: usize,
    pub(crate) history_rows: usize,
    pub(crate) ask_mode: bool,
    pub(crate) detail_view: Option<DetailView>,
    pub(crate) model_storage_warning: Option<String>,
    pub(crate) console_storage_warning: Option<String>,
    max_history_bytes: usize,
    diff_settings: Option<(usize, usize)>,
    max_detail_entries: usize,
    history_bytes: usize,
    history_revision: u64,
    pub(crate) history_omitted: usize,
    pub(crate) history_clipped_bytes: usize,
    wrapped_revision: u64,
    wrapped_columns: usize,
    wrapped_history: Arc<Vec<String>>,
    structured: Option<StructuredChoices>,
    recovery_report: Value,
    recovery_progress: String,
    changes: Vec<Value>,
    status_report: Value,
    details: VecDeque<Value>,
    last_answer: String,
    pub(crate) copy_requested: Option<String>,
    previewed_choice: Option<(ChoiceKind, String)>,
    pick: usize,
    reasoning_chars: usize,
    reasoning_text: String,
    folded_reasoning: Option<(usize, String)>,
    reasoning_open: bool,
    model_restored: Option<usize>,
    pub(crate) busy_at: Option<Instant>,
    pub(crate) task_step: Option<(String, usize)>,
    pub(crate) journal_hint: String,
    confirm_liberate: bool,
}

impl App {
    pub fn new(max_chars: usize, max_history: usize) -> Self {
        let mut app = Self {
            input: String::new(),
            cursor: 0,
            history: VecDeque::new(),
            selected: 0,
            dismissed: false,
            max_chars,
            max_history,
            status: "Consola local".into(),
            model_label: "sin seleccionar".into(),
            digest: String::new(),
            response: String::new(),
            busy: false,
            choosing: false,
            models: Vec::new(),
            action: None,
            proposals: Vec::new(),
            granting: false,
            task_mode: false,
            scroll: 0,
            sent: VecDeque::new(),
            recalled: None,
            draft: String::new(),
            context_used: None,
            context_limit: None,
            session_models: Vec::new(),
            exclusive: false,
            partial_effects: false,
            view_rows: 24,
            history_rows: 1,
            ask_mode: false,
            detail_view: None,
            model_storage_warning: None,
            console_storage_warning: None,
            max_history_bytes: usize::MAX,
            diff_settings: None,
            max_detail_entries: max_history,
            history_bytes: 0,
            history_revision: 0,
            history_omitted: 0,
            history_clipped_bytes: 0,
            wrapped_revision: u64::MAX,
            wrapped_columns: 0,
            wrapped_history: Arc::new(Vec::new()),
            structured: None,
            recovery_report: Value::Null,
            recovery_progress: String::new(),
            changes: Vec::new(),
            status_report: Value::Null,
            details: VecDeque::new(),
            last_answer: String::new(),
            copy_requested: None,
            previewed_choice: None,
            pick: 0,
            reasoning_chars: 0,
            reasoning_text: String::new(),
            folded_reasoning: None,
            reasoning_open: false,
            model_restored: None,
            busy_at: None,
            task_step: None,
            journal_hint: String::new(),
            confirm_liberate: false,
        };
        app.message(WELCOME);
        app
    }

    pub fn menu_open(&self) -> bool {
        !self.dismissed && self.command_input()
    }

    fn command_input(&self) -> bool {
        self.input.starts_with('/') && !self.input.chars().any(char::is_whitespace)
    }

    pub fn matches(&self) -> Vec<&'static Command> {
        if !self.menu_open() {
            return Vec::new();
        }
        COMMANDS
            .iter()
            .filter(|command| command.name.starts_with(&self.input))
            .collect()
    }

    pub fn configure_ui(&mut self, config: &crate::config::UiConfig) {
        self.max_history_bytes = config.max_history_bytes;
        self.diff_settings = Some((config.diff_context_lines, config.diff_max_cells));
        self.max_detail_entries = config.max_detail_entries;
        self.trim_history();
    }

    pub fn message(&mut self, text: impl Into<String>) {
        let mut text = text.into();
        if text.len() > self.max_history_bytes {
            let original = text.len();
            let mut end = self.max_history_bytes;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            self.history_clipped_bytes = self.history_clipped_bytes.saturating_add(original - end);
        }
        self.history_bytes = self.history_bytes.saturating_add(text.len());
        self.history.push_back(text);
        self.trim_history();
        self.history_revision = self.history_revision.wrapping_add(1);
    }

    fn trim_history(&mut self) {
        while self.history.len() > self.max_history || self.history_bytes > self.max_history_bytes {
            if let Some(removed) = self.history.pop_front() {
                self.history_bytes = self.history_bytes.saturating_sub(removed.len());
                self.history_omitted = self.history_omitted.saturating_add(1);
                self.history_revision = self.history_revision.wrapping_add(1);
            } else {
                break;
            }
        }
    }

    pub(crate) fn wrapped_history(&mut self, columns: usize) -> Arc<Vec<String>> {
        if self.wrapped_revision != self.history_revision || self.wrapped_columns != columns {
            self.wrapped_history = Arc::new(
                self.history
                    .iter()
                    .flat_map(|entry| super::render::wrap_text(entry, columns))
                    .collect(),
            );
            self.wrapped_revision = self.history_revision;
            self.wrapped_columns = columns;
        }
        self.wrapped_history.clone()
    }

    fn retain_detail(&mut self, value: Value) {
        self.details.push_back(value);
        while self.details.len() > self.max_detail_entries
            || self
                .details
                .iter()
                .map(|entry| entry.to_string().len())
                .sum::<usize>()
                > self.max_history_bytes
        {
            if self.details.pop_front().is_none() {
                break;
            }
        }
    }

    pub fn begin(&mut self, action: Action, status: &str) {
        self.busy = true;
        self.busy_at = Some(Instant::now());
        self.status = status.into();
        self.action = Some(action);
    }

    pub fn take_action(&mut self) -> Option<Action> {
        self.action.take()
    }

    fn open_detail(&mut self, title: &str, mut text: String) {
        if text.len() > self.max_history_bytes {
            let original = text.len();
            text.truncate(floor_char(&text, self.max_history_bytes));
            text.push_str(&format!(
                "\nVista recortada: {original} bytes en el resultado completo."
            ));
        }
        self.detail_view = Some(DetailView {
            title: title.into(),
            text,
            scroll: 0,
            highlights: Default::default(),
        });
    }

    fn open_change_detail(&mut self, title: &str, item: &Value) {
        let result = item_result(item);
        let Some((context, max_cells)) = self.diff_settings else {
            self.open_detail(title, status_detail(item));
            return;
        };
        let Some(change) = result.get("diff").and_then(Value::as_object) else {
            self.open_detail(title, status_detail(item));
            return;
        };
        let before = match change.get("before") {
            Some(Value::Null) => "",
            Some(Value::String(text)) => text,
            _ => {
                self.open_detail(title, status_detail(item));
                return;
            }
        };
        let Some(after) = change.get("after").and_then(Value::as_str) else {
            self.open_detail(title, status_detail(item));
            return;
        };
        let metadata: serde_json::Map<String, Value> = result
            .as_object()
            .expect("diff pertenece a objeto")
            .iter()
            .filter(|(key, _)| key.as_str() != "diff")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let mut text = status_detail(&Value::Object(metadata));
        if before.len().saturating_add(after.len()) > self.max_history_bytes {
            text.push_str("\nDiff omitido: las fuentes superan el limite de bytes de la vista. Conserva los SHA y el respaldo indicados.");
            self.open_detail(title, text);
            return;
        }
        let diff = super::diff::format(before, after, context, max_cells);
        text.push_str("\n\nDiff (+ añadido, - eliminado):\n");
        let offset = text.bytes().filter(|b| *b == b'\n').count();
        text.push_str(&diff.text);
        let cut = floor_char(&text, self.max_history_bytes);
        let content_lines = text[..cut].lines().count();
        let highlights = diff
            .highlights
            .into_iter()
            .filter_map(|(line, tone)| {
                let index = line + offset;
                (index < content_lines).then_some((index, tone))
            })
            .collect();
        self.open_detail(title, text);
        if let Some(view) = &mut self.detail_view {
            view.highlights = highlights;
        }
    }

    fn set_structured(&mut self, kind: ChoiceKind, items: Vec<Value>, explanation: &str) {
        self.choosing = false;
        self.granting = false;
        self.detail_view = None;
        self.previewed_choice = None;
        self.pick = 0;
        self.message(explanation);
        if items.is_empty() {
            self.message("No hay entradas disponibles en esta vista.");
        }
        self.structured = Some(StructuredChoices { kind, items });
    }

    fn detail_structured(&mut self, prompt: &str) {
        let Some(choices) = &self.structured else {
            return;
        };
        let Some(index) = self.list_index(prompt, choices.items.len()) else {
            self.message("Elige una fila válida; no se ejecutó ninguna acción.");
            return;
        };
        self.pick = index;
        let kind = choices.kind;
        let item = choices.items[index].clone();
        self.previewed_choice = Some((kind, choice_key(kind, &item)));
        let title = structured_label(kind, &item);
        if matches!(kind, ChoiceKind::Changes | ChoiceKind::Undo) {
            self.open_change_detail(&title, &item);
        } else {
            self.open_detail(&title, status_detail(&item));
        }
    }

    fn submit_structured(&mut self, prompt: &str) {
        let Some(choices) = &self.structured else {
            return;
        };
        let Some(index) = self.list_index(prompt, choices.items.len()) else {
            self.message("Elige una fila válida; no se ejecutó ninguna acción.");
            return;
        };
        let kind = choices.kind;
        let item = choices.items[index].clone();
        self.pick = index;
        if kind == ChoiceKind::Changes {
            self.detail_structured(prompt);
            return;
        }
        if matches!(
            kind,
            ChoiceKind::Recovery | ChoiceKind::Undo | ChoiceKind::Archive | ChoiceKind::Profiles
        ) && self.previewed_choice.as_ref() != Some(&(kind, choice_key(kind, &item)))
        {
            self.detail_structured(prompt);
            self.message(
                "Revisa la evidencia de esta fila; Ctrl+Enter confirma y Esc vuelve sin actuar.",
            );
            return;
        }
        match kind {
            ChoiceKind::Recovery => {
                if item["needs_review"] != true {
                    self.message(
                        "Esta entrada no necesita revisión; no se repitió ninguna operación.",
                    );
                    return;
                }
                if let Some(fingerprint) = item["fingerprint"]
                    .as_str()
                    .filter(|value| !value.is_empty())
                {
                    self.begin(
                        Action::AcknowledgeRecovery {
                            fingerprint: fingerprint.into(),
                        },
                        "Registrando revisión humana de evidencia vigente…",
                    );
                } else {
                    self.message(
                        "No hay huella vigente para revisar; vuelve a consultar /recover.",
                    );
                }
            }
            ChoiceKind::Undo => {
                if let Some((path, expected_sha256, backup_path, backup_sha256)) =
                    undo_fields(&item)
                {
                    self.begin(
                        Action::Undo {
                            path,
                            expected_sha256,
                            backup_path,
                            backup_sha256,
                        },
                        "Comprobando versión actual y restaurando el respaldo seleccionado…",
                    );
                } else {
                    self.message("Este resultado no contiene un respaldo y versiones restaurables; no se ha cambiado nada.");
                }
            }
            ChoiceKind::Profiles => {
                if let Some(name) = item["id"]
                    .as_str()
                    .or_else(|| item["name"].as_str())
                    .filter(|name| !name.is_empty())
                {
                    self.begin(
                        Action::SelectProfile(name.into()),
                        "Validando el perfil elegido…",
                    );
                } else {
                    self.message("Perfil sin identificador; no se cambió la configuración.");
                }
            }
            ChoiceKind::Archive => {
                if let Some(fingerprint) = item["fingerprint"]
                    .as_str()
                    .filter(|value| !value.is_empty())
                {
                    self.begin(
                        Action::ConfirmArchive {
                            fingerprint: fingerprint.into(),
                        },
                        "Archivando exclusivamente la evidencia revisada…",
                    );
                } else {
                    self.message("No hay huella de archivo vigente; vuelve a consultar /archive.");
                }
            }
            ChoiceKind::Changes => {}
        }
    }

    fn request_copy(&mut self) {
        let text = self
            .detail_view
            .as_ref()
            .map(|view| view.text.clone())
            .unwrap_or_else(|| self.last_answer.clone());
        if text.is_empty() {
            self.message("No hay respuesta completa para copiar.");
        } else {
            self.copy_requested = Some(text);
        }
    }

    pub(crate) fn mode_label(&self) -> String {
        if let Some((role, step)) = &self.task_step
            && self.task_mode
        {
            return format!("Tarea · {role} · paso {step}");
        }
        if self.task_mode {
            "Tarea".into()
        } else if self.ask_mode {
            "Preguntar · solo lectura".into()
        } else {
            "Editar".into()
        }
    }

    pub(crate) fn recovery_heading(&self) -> Option<&str> {
        self.structured
            .as_ref()
            .filter(|choices| choices.kind == ChoiceKind::Recovery)
            .map(|_| self.recovery_progress.as_str())
    }

    pub(crate) fn footer(&self) -> &'static str {
        if self.detail_view.is_some() {
            if self
                .structured
                .as_ref()
                .is_some_and(|choices| choices.kind == ChoiceKind::Profiles)
            {
                return "PgUp/PgDn · Ctrl+Enter aplicar perfil · Esc volver";
            }

            if self.granting
                || self.structured.as_ref().is_some_and(|choices| {
                    matches!(
                        choices.kind,
                        ChoiceKind::Recovery | ChoiceKind::Undo | ChoiceKind::Archive
                    )
                })
            {
                return "PgUp/PgDn · Ctrl+Y copiar · Ctrl+Enter confirmar · Esc volver";
            }
            return "PgUp/PgDn desplazar · Ctrl+Y copiar · Esc volver";
        }
        if self.confirm_liberate {
            return "/ comandos · Ctrl+Enter descargar · Enter detalle · Esc cancelar";
        }
        if self.menu_open() {
            return "/ comandos · Flechas elegir · Enter ejecutar · Esc cerrar";
        }
        if self.granting {
            return "/ comandos · Enter detalle · Ctrl+Enter conceder/retirar · Esc cerrar";
        }
        if self.choosing {
            return "/ comandos · Elige un número · Ctrl+Enter elegir · Esc cancelar";
        }
        if let Some(choices) = &self.structured {
            if choices.items.is_empty() {
                return "/ comandos · Sin filas disponibles · Esc cerrar";
            }
            return match choices.kind {
                ChoiceKind::Changes => {
                    "/ comandos · Flechas elegir · Enter ver diff/evidencia · Esc cerrar"
                }
                ChoiceKind::Profiles => {
                    "/ comandos · Enter detalle · Ctrl+Enter aplicar perfil · Esc cerrar"
                }
                _ => "/ comandos · Enter evidencia · Ctrl+Enter confirmar fila · Esc cerrar",
            };
        }
        "/ comandos · Ctrl+Enter enviar · Enter línea · Esc cancelar · Ctrl+Y copiar"
    }

    pub(crate) fn load_local_history(&mut self, files: &crate::files::Files) {
        let stored = match files.read_private_checked_with("console.json", |bytes| {
            serde_json::from_slice::<ConsoleHistory>(bytes).is_ok()
        }) {
            Ok(value) => value,
            Err(error) => {
                let warning = format!("No se pudo leer el historial local de la consola: {error}");
                self.console_storage_warning = Some(warning.clone());
                self.message(warning);
                return;
            }
        };
        if let Some(warning) = stored.warning {
            self.console_storage_warning = Some(warning.clone());
            self.message(warning);
        }
        let Some(bytes) = stored.bytes else {
            return;
        };
        let Ok(saved) = serde_json::from_slice::<ConsoleHistory>(&bytes) else {
            self.message("No se pudo leer el historial local de la consola.");
            return;
        };
        let mut loaded = false;
        for line in saved.lines {
            if line != WELCOME && line != LOCAL_HISTORY_MARK && !line.starts_with(REASONING_MARK) {
                self.message(line);
                loaded = true;
            }
        }
        if !saved.reasoning.is_empty() {
            self.folded_reasoning = Some((saved.reasoning_chars.max(1), saved.reasoning));
            loaded = true;
        }
        if loaded {
            self.message(LOCAL_HISTORY_MARK);
        }
    }

    pub(crate) fn save_local_history(&mut self, files: &crate::files::Files) {
        let lines: Vec<String> = self
            .history
            .iter()
            .filter(|line| {
                line.as_str() != WELCOME
                    && line.as_str() != LOCAL_HISTORY_MARK
                    && !line.starts_with(REASONING_MARK)
            })
            .cloned()
            .collect();
        let reasoning = if !self.reasoning_text.is_empty() {
            self.reasoning_text.clone()
        } else {
            self.folded_reasoning
                .as_ref()
                .map(|(_, text)| text.clone())
                .unwrap_or_default()
        };
        let reasoning_chars = if self.reasoning_chars > 0 {
            self.reasoning_chars
        } else {
            self.folded_reasoning
                .as_ref()
                .map(|(chars, _)| *chars)
                .unwrap_or(0)
        };
        if lines.is_empty() && reasoning.is_empty() {
            return;
        }
        let saved = ConsoleHistory {
            lines,
            reasoning_chars,
            reasoning,
        };
        let Ok(bytes) = serde_json::to_vec(&saved) else {
            return;
        };
        match files.write_private("console.json", &bytes) {
            Ok(()) => self.console_storage_warning = None,
            Err(error) => {
                let warning = format!("Historial de consola sin guardar: {error}");
                if self.console_storage_warning.as_ref() != Some(&warning) {
                    self.message(warning.clone());
                }
                self.console_storage_warning = Some(warning);
            }
        }
    }

    pub fn event(&mut self, event: Event) {
        match event {
            Event::StorageWarning(text) => {
                self.model_storage_warning = (!text.is_empty()).then_some(text.clone());
                if !text.is_empty() {
                    self.message(text);
                }
                return;
            }
            Event::Status { report } => {
                self.status_report = report.clone();
                self.open_detail("Estado de la sesión", status_detail(&report));
                self.status = self.idle_status();
            }
            Event::Recovery { report } | Event::RecoveryReviewed { report } => {
                self.partial_effects = report["needs_review"].as_bool().unwrap_or(true);
                self.recovery_report = report.clone();
                let items = report["entries"].as_array().cloned().unwrap_or_default();
                self.recovery_progress = recovery_progress(&items, self.partial_effects);
                self.set_structured(ChoiceKind::Recovery,items,"Recuperación: Enter muestra evidencia; Ctrl+Enter registra revisión humana de la fila vigente.");
                self.status = self.idle_status();
            }
            Event::Changes { items } => {
                let undo = self
                    .structured
                    .as_ref()
                    .is_some_and(|choices| choices.kind == ChoiceKind::Undo);
                self.changes = items;
                let visible = if undo {
                    self.changes
                        .iter()
                        .filter(|item| undo_fields(item).is_some())
                        .cloned()
                        .collect()
                } else if self.changes.is_empty() {
                    self.details.iter().cloned().collect()
                } else {
                    self.changes.clone()
                };
                self.set_structured(if undo {ChoiceKind::Undo} else {ChoiceKind::Changes},visible,
                    if undo {"Restaurar: revisa la fila con Enter. Ctrl+Enter confirma ruta, versión actual y respaldo."} else {"Cambios observados: flechas eligen y Enter muestra diff y evidencia."});
                self.status = self.idle_status();
            }
            Event::UndoComplete { result } => {
                if result["status"] == "verified" {
                    self.message(
                        "Restauración verificada; el contenido anterior también quedó respaldado.",
                    );
                } else {
                    self.message(format!("Restauración no verificada: {}", result));
                }
                if matches!(
                    result["status"].as_str(),
                    Some("partial" | "unknown" | "applied")
                ) {
                    self.partial_effects = true;
                }
                self.retain_detail(json!({"name":"restore_backup","result":result}));
                self.structured = None;
                self.status = self.idle_status();
            }
            Event::Profiles(items) => {
                self.set_structured(ChoiceKind::Profiles,items,"Perfil: Enter muestra parámetros; Ctrl+Enter aplica tu elección sin cambiar el modelo.");
                self.status = self.idle_status();
            }
            Event::ProfileSelected { report } => {
                self.structured = None;
                self.status_report = report.clone();
                self.message(format!("Perfil aplicado: {}", profile_label(&report)));
                self.open_detail("Perfil seleccionado", status_detail(&report));
                self.status = self.idle_status();
            }
            Event::Archive { report } | Event::ArchiveComplete { report } => {
                let items = report["entries"]
                    .as_array()
                    .or_else(|| report["items"].as_array())
                    .cloned()
                    .unwrap_or_default();
                self.set_structured(ChoiceKind::Archive,items,"Archivo: Enter muestra evidencia y destino. Ctrl+Enter confirma únicamente la fila revisada.");
                if let Some(summary) = report["summary"].as_str() {
                    self.message(summary);
                }
                for key in ["project_scan", "retention"] {
                    if let Some(error) = report[key]["error"].as_str() {
                        self.message(format!("{key}: {error}"));
                    }
                }
                if report["status"] == "verified"
                    && let Some(destination) = report["archived"]["destination"].as_str()
                {
                    self.message(format!("Archivado verificado: {destination}"));
                }
                self.status = self.idle_status();
            }
            Event::Budget { used, limit } => {
                self.context_used = Some(used);
                self.context_limit = Some(limit);
                return;
            }
            Event::Activity { role, text } => {
                self.fold_reasoning();
                self.status = format!("{role} · {text}");
                return;
            }
            Event::Reasoning { chars, text } => {
                self.reasoning_chars = chars;
                self.reasoning_text = text;
                self.status = format!("razonando · {chars} caracteres");
                if self.reasoning_open {
                    self.close_reasoning();
                    self.reasoning_open = true;
                    self.message(format!(
                        "{REASONING_MARK}\n{}",
                        clip_reasoning(&self.reasoning_text)
                    ));
                }
                return;
            }
            Event::Restored { dropped } => {
                self.model_restored = Some(dropped);
                return;
            }
            Event::TaskStep { role, step } => {
                self.task_mode = true;
                self.task_step = Some((role, step));
                return;
            }
            Event::Tool {
                role,
                name,
                arguments,
                result,
                journal,
                ..
            } => {
                self.fold_reasoning();
                if !journal.is_empty() {
                    self.journal_hint = journal;
                }
                let state = result
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("sin estado");
                if matches!(state, "partial" | "unknown" | "applied") {
                    self.partial_effects = true;
                    self.status = self.idle_status();
                }
                let extra = tool_extra(&name, &arguments, &result);
                self.message(format!("Acción [{role}]: {name} · {state} · {extra}"));
                self.retain_detail(
                    json!({"role":role,"name":name,"arguments":arguments,"result":result}),
                );
                return;
            }
            Event::Notice { role, text } => {
                self.fold_reasoning();
                if !self.response.is_empty() {
                    let progress = std::mem::take(&mut self.response);
                    self.message(format!("\nRespuesta del agente (progreso):\n{progress}"));
                }
                self.message(format!("Agente [{role}]:\n{text}"));
                if role == "sistema" {
                    if text.contains("Efectos bloqueados:") {
                        self.partial_effects = true;
                        self.journal_hint = ".pillcli/sessions".into();
                    }
                    self.note_exclusive(&text);
                    self.apply_unloaded(&text);
                    self.status = if self.partial_effects {
                        self.idle_status()
                    } else {
                        liberation_status(&text).into()
                    };
                    return;
                }
                self.status = if self.busy {
                    format!("Trabajando · {role} · Esc cancela")
                } else {
                    self.idle_status()
                };
                return;
            }
            Event::Permissions(proposals) => {
                self.structured = None;
                self.choosing = false;
                self.granting = !proposals.is_empty();
                self.message("Flechas marcan una fila. Enter muestra el detalle. Ctrl+Enter concede o retira. Esc cierra sin cambiar nada.");
                for (index, proposal) in proposals.iter().enumerate() {
                    self.message(format!(
                        "{} · {} · {}",
                        index + 1,
                        proposal_label(proposal),
                        proposal_state(&proposal.id, &proposal.detail)
                    ));
                }
                if proposals.is_empty() {
                    self.message("No hay propuestas. commands.commands, mcp.servers y gpu.exclusive viven en la configuración; eso no concede permisos.");
                }
                self.proposals = proposals;
                self.pick = 0;
                self.status = "Propuestas · consentimiento explícito".into();
            }
            Event::Liberated(text) => {
                self.note_exclusive(&text);
                self.apply_unloaded(&text);
                self.status = liberation_status(&text).into();
                self.message(text);
            }
            Event::Revoked(id) => {
                self.mark_grant(&id, false);
                if id == "exclusive" {
                    self.exclusive = false;
                }
                self.message(format!("Concesión retirada: {id}"));
                self.status = "Concesión retirada".into();
            }
            Event::Granted(id) => {
                self.granting = false;
                self.mark_grant(&id, true);
                self.message(format!(
                    "Concesión guardada para {id}; cambios de definición/scripts la invalidan."
                ));
                if id.starts_with("mcp:") {
                    self.status = "Servidor MCP concedido".into();
                } else if id == "exclusive" {
                    self.exclusive = true;
                    self.status = "Exclusividad concedida".into();
                } else if id.starts_with("command:") {
                    self.status = "Comando concedido".into();
                } else {
                    self.status = "Concesión guardada".into();
                }
            }
            Event::Delta(text) => {
                self.fold_reasoning();
                if self.busy && self.status.starts_with("razonando") {
                    self.status = "Generando · Esc cancela".into();
                }
                self.response.push_str(&text);
                return;
            }
            Event::Models(models) => {
                self.structured = None;
                self.models = models;
                self.choosing = true;
                self.pick = 0;
                self.message("Elige un número y pulsa Ctrl+Enter (sin descargas):");
                let labels: Vec<_> = self
                    .models
                    .iter()
                    .enumerate()
                    .map(|(i, model)| {
                        format!(
                            "{} · {} · {} · {}",
                            i + 1,
                            model.name,
                            size_label(model.size),
                            model.memory_label()
                        )
                    })
                    .collect();
                for label in labels {
                    self.message(label);
                }
                if self.models.is_empty() {
                    self.message("No hay modelos instalados. /model permite consultar de nuevo.");
                }
                self.status = "Selección explícita de modelo".into();
            }
            Event::Selected(model) => {
                self.choosing = false;
                if !self
                    .session_models
                    .iter()
                    .any(|item| item == &model.model.name)
                {
                    self.session_models.push(model.model.name.clone());
                }
                self.model_label = model.model.name;
                self.digest = model.model.digest;
                self.message(format!(
                    "Capacidades: {} · contexto máximo anunciado: {}",
                    model.capabilities.join(", "),
                    model.context_limit
                ));
                if !model.capabilities.iter().any(|c| c == "tools") {
                    self.message("El modelo no anuncia herramientas; solo chat de texto.");
                }
                if let Some(dropped) = self.model_restored.take() {
                    if dropped > 0 {
                        self.message(format!("Conversación recuperada con este modelo. Al guardar se dejaron fuera {dropped} mensajes antiguos porque no cabían en el diario; el modelo no los tiene."));
                    } else {
                        self.message("Conversación recuperada con este modelo. El historial que el modelo ya vio vuelve a estar cargado.");
                    }
                } else {
                    self.message(
                        "Nueva conversación. Ctrl+Enter envía; Enter añade línea. Esc cancela.",
                    );
                }
                self.status = self.idle_status();
            }
            Event::Journal(text) => {
                self.message(text);
                self.status = self.idle_status();
            }
            Event::Complete {
                elapsed_ms,
                first_token_ms,
                metrics,
                answer,
            } => {
                let streamed = std::mem::take(&mut self.response);
                let response = answer.unwrap_or(streamed);
                self.last_answer = response.clone();
                self.message(format!("\nRespuesta:\n{response}"));
                if let (Some(prompt), Some(generated), Some(load_ns)) = (
                    metrics
                        .get("prompt_eval_count")
                        .and_then(serde_json::Value::as_u64),
                    metrics
                        .get("eval_count")
                        .and_then(serde_json::Value::as_u64),
                    metrics
                        .get("load_duration")
                        .and_then(serde_json::Value::as_u64),
                ) {
                    self.message(format!(
                        "Ollama: {prompt} tokens de entrada · {generated} generados · carga {} ms",
                        load_ns / 1_000_000
                    ));
                }
                self.status = if self.partial_effects {
                    self.idle_status()
                } else {
                    format!(
                        "Respuesta completa · {elapsed_ms} ms · primer texto {} ms",
                        first_token_ms.unwrap_or(0)
                    )
                };
            }
            Event::Failed(error) => {
                self.fold_reasoning();
                self.partial();
                self.message(format!("Error: {error}"));
                if error.contains("efectos parciales") {
                    self.partial_effects = true;
                }
                self.status = if error.contains("efectos parciales") {
                    self.idle_status()
                } else if error.contains("historial demasiado grande") || error.contains("contexto")
                {
                    "Contexto agotado · /model inicia otra conversación".into()
                } else if self.partial_effects {
                    self.idle_status()
                } else if self.digest.is_empty() {
                    "Operación fallida · /model para consultar o elegir".into()
                } else {
                    "Operación fallida · puedes volver a intentarlo".into()
                };
            }
            Event::Cancelled => {
                self.fold_reasoning();
                self.partial();
                self.message("Operación cancelada; no se añade al historial del modelo.");
                self.status = if self.partial_effects {
                    self.idle_status()
                } else {
                    "Cancelado · puedes volver a intentarlo".into()
                };
            }
        }
        self.busy = false;
        self.busy_at = None;
    }

    fn partial(&mut self) {
        if !self.response.is_empty() {
            let response = std::mem::take(&mut self.response);
            self.message(format!("\nRespuesta parcial NO confirmada:\n{response}"));
        }
    }

    fn submit(&mut self) {
        if self.busy {
            self.message("Hay una operación activa. Esc la cancela.");
            return;
        }
        if self.confirm_liberate {
            self.confirm_liberate = false;
            self.begin(Action::Liberate, "Comprobando memoria de esta sesión…");
            self.finish_edit();
            return;
        }
        let prompt = self.input.trim().to_owned();
        if self.structured.is_some() {
            self.submit_structured(&prompt);
            self.finish_edit();
            return;
        } else if self.granting {
            let Some(index) = self.list_index(&prompt, self.proposals.len()) else {
                self.message("Elige una propuesta válida; ninguna se concedió.");
                return;
            };
            let id = self.proposals[index].id.clone();
            let fingerprint = self.proposals[index].fingerprint.clone();
            let state = proposal_state(&id, &self.proposals[index].detail);
            if state == "concedido" {
                self.begin(Action::Revoke { id }, "Retirando concesión…");
            } else if state == "rechazada" {
                self.message("una raíz extra no se concede ni se retira");
                return;
            } else {
                self.begin(
                    Action::Grant { id, fingerprint },
                    "Guardando concesión explícita…",
                );
            }
        } else if self.choosing {
            let Some(index) = self.list_index(&prompt, self.models.len()) else {
                self.message("Elige un número válido de la lista con Ctrl+Enter.");
                return;
            };
            let name = self.models[index].name.clone();
            self.begin(Action::Select(name), "Comprobando modelo y capacidades…");
        } else if prompt.is_empty() {
            return;
        } else if self.digest.is_empty() {
            self.message("Selecciona primero un modelo con /model.");
            return;
        } else {
            self.message(format!("\nTú:\n{prompt}"));
            self.response.clear();
            self.sent.push_back(prompt.clone());
            while self.sent.len() > self.max_history {
                self.sent.pop_front();
            }
            self.recalled = None;
            self.draft.clear();
            self.begin(
                if self.task_mode {
                    Action::Task(prompt)
                } else if self.ask_mode {
                    Action::Ask(prompt)
                } else {
                    Action::Chat(prompt)
                },
                "Generando · Esc cancela · Ctrl+C sale",
            );
        }
        self.finish_edit();
    }

    /// Devuelve true únicamente cuando se solicita salir.
    pub fn key(&mut self, key: Key) -> bool {
        if key == Key::Interrupt {
            return true;
        }
        if key == Key::Copy {
            self.request_copy();
            return false;
        }
        if self.detail_view.is_some() {
            match key {
                Key::Escape => self.detail_view = None,
                Key::PageUp => {
                    let page = self.page_rows();
                    if let Some(view) = &mut self.detail_view {
                        view.scroll = view.scroll.saturating_sub(page);
                    }
                }
                Key::PageDown => {
                    let page = self.page_rows();
                    if let Some(view) = &mut self.detail_view {
                        view.scroll = view.scroll.saturating_add(page);
                    }
                }
                Key::Home => {
                    if let Some(view) = &mut self.detail_view {
                        view.scroll = 0;
                    }
                }
                Key::Submit if self.granting || self.structured.is_some() => {
                    self.detail_view = None;
                    self.submit();
                }
                _ => {}
            }
            return false;
        }
        if key == Key::AltP && !self.busy {
            self.input.clear();
            self.cursor = 0;
            self.task_mode = false;
            self.ask_mode = true;
            self.task_step = None;
            self.message(
                "Modo Preguntar: herramientas de solo lectura, sin editar ni ejecutar comandos.",
            );
            self.status = self.idle_status();
            return false;
        }
        if key == Key::AltE && !self.busy {
            self.input.clear();
            self.cursor = 0;
            self.task_mode = false;
            self.ask_mode = false;
            self.task_step = None;
            self.message("Modo chat con herramientas · Editar; acciones y resultados visibles.");
            self.status = self.idle_status();
            return false;
        }
        if key == Key::AltT && !self.busy {
            self.input.clear();
            self.cursor = 0;
            self.task_mode = true;
            self.ask_mode = false;
            self.task_step = None;
            self.message("Modo tarea: próxima petición usa coordinador, explorador, implementador, tester y reviewer. Necesita pruebas concedidas.");
            self.status = self.idle_status();
            return false;
        }
        match key {
            Key::Interrupt => return true,
            Key::Text(value, repeat) if !value.is_control() => {
                for _ in 0..repeat {
                    if self.input.chars().count() >= self.max_chars {
                        break;
                    }
                    self.input.insert(self.cursor, value);
                    self.cursor += value.len_utf8();
                }
                self.edited();
            }
            Key::Backspace if self.cursor > 0 => {
                let previous = prev_boundary(&self.input, self.cursor);
                self.input.drain(previous..self.cursor);
                self.cursor = previous;
                self.edited();
            }
            Key::Delete if self.cursor < self.input.len() => {
                let next = next_boundary(&self.input, self.cursor);
                self.input.drain(self.cursor..next);
                self.edited();
            }
            Key::Left if self.cursor > 0 => self.cursor = prev_boundary(&self.input, self.cursor),
            Key::Right if self.cursor < self.input.len() => {
                self.cursor = next_boundary(&self.input, self.cursor)
            }
            Key::ToggleReasoning => self.toggle_reasoning(),
            Key::WordLeft => self.cursor = word_left(&self.input, self.cursor),
            Key::WordRight => self.cursor = word_right(&self.input, self.cursor),
            Key::Home => self.cursor = line_start(&self.input, self.cursor),
            Key::End => self.cursor = line_end(&self.input, self.cursor),
            Key::Escape => {
                if self.confirm_liberate && !self.busy {
                    self.confirm_liberate = false;
                    self.message("Descarga cancelada.");
                    return false;
                }
                self.dismissed = true;
                self.granting = false;
                self.choosing = false;
                self.structured = None;
                if self.busy {
                    self.action = Some(Action::Cancel);
                }
            }
            Key::Submit if !self.command_input() => self.submit(),
            Key::Up | Key::Down => {
                let count = self.matches().len();
                if count > 0 {
                    self.selected = if key == Key::Up {
                        (self.selected + count - 1) % count
                    } else {
                        (self.selected + 1) % count
                    };
                } else if self.choice_len() > 0 {
                    let count = self.choice_len();
                    self.pick = if key == Key::Up {
                        (self.pick + count - 1) % count
                    } else {
                        (self.pick + 1) % count
                    };
                } else if let Some(next) = self.move_line(key == Key::Down) {
                    self.cursor = next;
                } else if !self.sent.is_empty() && !self.choosing && !self.granting {
                    if self.recalled.is_none() {
                        self.draft = self.input.clone();
                    }
                    let last = self.sent.len() - 1;
                    self.recalled = match (key, self.recalled) {
                        (Key::Up, None) => Some(last),
                        (Key::Up, Some(i)) => Some(i.saturating_sub(1)),
                        (Key::Down, Some(i)) if i < last => Some(i + 1),
                        _ => None,
                    };
                    self.input = self
                        .recalled
                        .map(|i| self.sent[i].clone())
                        .unwrap_or_else(|| self.draft.clone());
                    self.cursor = self.input.len();
                }
            }
            Key::PageUp => self.scroll = self.scroll.saturating_add(self.page_rows()),
            Key::PageDown => self.scroll = self.scroll.saturating_sub(self.page_rows()),
            Key::Tab => {
                if let Some(command) = self.matches().get(self.selected).copied() {
                    self.input = command.name.into();
                    self.cursor = self.input.len();
                    self.selected = 0;
                }
            }
            Key::Enter => {
                if self.confirm_liberate {
                    self.message(
                        "Ctrl+Enter confirma la descarga. Enter no descarga; Esc cancela.",
                    );
                    return false;
                }
                if self.structured.is_some() && !self.command_input() {
                    let prompt = self.input.trim().to_owned();
                    self.detail_structured(&prompt);
                    self.finish_edit();
                    return false;
                }
                if self.granting && !self.command_input() {
                    let prompt = self.input.trim().to_owned();
                    let index = self.list_index(&prompt, self.proposals.len());
                    if let Some(proposal) = index.and_then(|value| self.proposals.get(value)) {
                        let title = proposal_label(proposal);
                        let detail = permission_detail(proposal);
                        self.open_detail(&title, detail);
                        self.finish_edit();
                    } else {
                        self.message("Elige una propuesta válida; ninguna se concedió.");
                    }
                    return false;
                }
                if !self.command_input() {
                    if self.input.chars().count() < self.max_chars {
                        self.input.insert(self.cursor, '\n');
                        self.cursor += 1;
                    }
                    return false;
                }
                let chosen = self
                    .matches()
                    .get(self.selected)
                    .map(|command| command.name.to_owned())
                    .unwrap_or_else(|| self.input.trim().to_owned());
                self.input.clear();
                self.cursor = 0;
                self.edited();
                match chosen.as_str() {
                    "/exit" => return true,
                    "/help" => {
                        self.message("Comandos disponibles (escribe / y usa flechas/Tab/Enter):");
                        self.message("/help · Mostrar esta ayuda");
                        self.message("/exit · Salir y restaurar la consola");
                        self.message("/model · Elegir modelo (Ctrl+Enter para confirmar)");
                        self.message("/permissions · Conceder/retirar comandos y MCP (Enter: detalle, Ctrl+Enter: concede/retira)");
                        self.message(
                            "/task · Modo tarea multiagente (requiere pruebas concedidas)",
                        );
                        self.message("/chat · Chat con herramientas (alias /edit)");
                        self.message("/ask · Solo lectura (sin herramientas de escritura)");
                        self.message("/status · Modelo, configuración, recursos y cuotas");
                        self.message("/profile · Elegir perfil de contexto/generación");
                        self.message("/recover · Inspeccionar evidencia durable");
                        self.message("/changes · Cambios verificados");
                        self.message("/undo · Restaurar respaldo (coincide SHA actual)");
                        self.message("/archive · Archivar evidencia revisada");
                        self.message("/copy · Copiar respuesta/detalle visible");
                        self.message("/journal · Leer diario local");
                        self.message("/liberate · Liberar modelos de esta sesión (Enter si exclusividad concedida)");
                        self.message("Teclas: Ctrl+Enter envía; Enter añade línea; Esc cancela; PgUp/PgDn desplaza; flechas recuperan peticiones; Ctrl+R pliega razonamiento; Inicio/Fin en línea; Ctrl+flechas saltan palabras.");
                    }
                    "/permissions" if !self.busy => {
                        self.begin(Action::Permissions, "Preparando propuestas completas…")
                    }
                    "/task" if !self.busy => {
                        self.task_mode = true;
                        self.ask_mode = false;
                        self.message("Modo tarea: próxima petición usa coordinador, explorador, implementador, tester y reviewer. Necesita pruebas concedidas.");
                        self.status = self.idle_status();
                    }
                    "/chat" | "/edit" if !self.busy => {
                        self.task_mode = false;
                        self.ask_mode = false;
                        self.task_step = None;
                        self.message(
                            "Modo chat con herramientas · Editar; acciones y resultados visibles.",
                        );
                        self.status = self.idle_status();
                    }
                    "/ask" if !self.busy => {
                        self.task_mode = false;
                        self.ask_mode = true;
                        self.task_step = None;
                        self.message("Modo Preguntar: herramientas de solo lectura, sin editar ni ejecutar comandos.");
                        self.status = self.idle_status();
                    }
                    "/status" if !self.busy => {
                        self.begin(Action::Status, "Consultando estado y cuotas…")
                    }
                    "/recover" if !self.busy => {
                        self.begin(Action::Recovery, "Inspeccionando evidencia durable…")
                    }
                    "/changes" if !self.busy => {
                        self.structured = None;
                        self.begin(Action::Changes, "Leyendo cambios observados…");
                    }
                    "/undo" if !self.busy => {
                        self.structured = Some(StructuredChoices {
                            kind: ChoiceKind::Undo,
                            items: Vec::new(),
                        });
                        self.begin(Action::Changes, "Buscando respaldos restaurables…");
                    }
                    "/profile" if !self.busy => {
                        self.begin(Action::Profiles, "Consultando perfiles disponibles…")
                    }
                    "/archive" if !self.busy => {
                        self.begin(Action::Archive, "Inspeccionando evidencia revisada…")
                    }
                    "/copy" => {
                        self.request_copy();
                    }
                    "/gpu" if !self.busy => {
                        use std::process::Command;
                        let out = Command::new("nvidia-smi")
                            .args([
                                "--query-gpu=memory.used,memory.free,memory.total",
                                "--format=csv,noheader,nounits",
                            ])
                            .output();
                        match out {
                            Ok(o) if o.status.success() => {
                                let text = String::from_utf8_lossy(&o.stdout);
                                let line = text.lines().next().unwrap_or("—").trim();
                                self.message(format!("VRAM (solo lectura, sin atribución): {line} MiB"));
                            }
                            _ => self.message("No disponible: nvidia-smi no encontrado. Solo lectura, sin modificar GPU."),
                        }
                    }
                    "/init" if !self.busy => {
                        let config_path = std::env::current_dir()
                            .map(|p| p.join("config/defaults.json"))
                            .unwrap_or_else(|_| std::path::PathBuf::from("config/defaults.json"));
                        let config_exists = config_path.exists();
                        let path_has_bin = std::env::var("PATH").map_or(false, |p| {
                            p.contains("PILLCLITER\\bin") || p.contains("PILLCLITER/bin")
                        });
                        let agents_path = std::path::PathBuf::from(
                            std::env::var("USERPROFILE").unwrap_or_default(),
                        )
                        .join(".agents/AGENTS.md");
                        let agents_exists = agents_path.exists();
                        let msg_config = if config_exists {
                            "PASS: defaults.json"
                        } else {
                            "WARN: defaults.json no encontrado"
                        };
                        let msg_path = if path_has_bin {
                            "PASS: PATH persistente"
                        } else {
                            "WARN: bin no en PATH persistente"
                        };
                        let msg_agents = if agents_exists {
                            format!(
                                "PASS: AGENTS.md encontrado ({} bytes)",
                                std::fs::metadata(&agents_path)
                                    .map(|m| m.len())
                                    .unwrap_or(0)
                            )
                        } else {
                            "WARN: AGENTS.md no encontrado".into()
                        };
                        self.message(format!("/init · Configuración: {msg_config} · PATH: {msg_path} · OpenCode: {msg_agents}"));
                    }
                    "/model" if !self.busy => self.begin(Action::Discover, "Consultando Ollama…"),
                    "/journal" if !self.busy => self.begin(Action::Journal, "Leyendo el diario…"),
                    "/liberate" if !self.busy => self.confirm_or_liberate(),
                    "" => {}
                    _ => self.message(format!(
                        "Comando no disponible: {chosen}. Escribe / para ver el listado."
                    )),
                }
            }
            _ => {}
        }
        false
    }

    fn edited(&mut self) {
        self.dismissed = false;
        self.selected = 0;
    }

    fn finish_edit(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.edited();
        self.scroll = 0;
    }

    fn confirm_or_liberate(&mut self) {
        if self.exclusive && !self.session_models.is_empty() {
            self.confirm_liberate = true;
            let names = self.session_models.join(", ");
            self.message(format!(
                "Ctrl+Enter confirma descargar: {names}. Enter no descarga; Esc cancela."
            ));
        } else {
            self.begin(Action::Liberate, "Comprobando memoria de esta sesión…");
        }
    }

    fn list_index(&self, prompt: &str, len: usize) -> Option<usize> {
        if len == 0 {
            return None;
        }
        if prompt.is_empty() {
            return (self.pick < len).then_some(self.pick);
        }
        prompt
            .parse::<usize>()
            .ok()
            .and_then(|value| value.checked_sub(1))
            .filter(|index| *index < len)
    }

    fn choice_len(&self) -> usize {
        if self.menu_open() || self.detail_view.is_some() {
            0
        } else if self.choosing {
            self.models.len()
        } else if self.granting {
            self.proposals.len()
        } else if let Some(choices) = &self.structured {
            choices.items.len()
        } else {
            0
        }
    }

    pub(crate) fn choice_rows(&self, max_rows: usize) -> Vec<String> {
        if self.detail_view.is_some() {
            return Vec::new();
        }
        let items: Vec<String> = if self.choosing {
            self.models
                .iter()
                .enumerate()
                .map(|(index, model)| {
                    format!(
                        "{} · {} · {} · {}",
                        index + 1,
                        model.name,
                        size_label(model.size),
                        model.memory_label()
                    )
                })
                .collect()
        } else if self.granting {
            self.proposals
                .iter()
                .enumerate()
                .map(|(index, proposal)| {
                    format!(
                        "{} · {} · {}",
                        index + 1,
                        proposal_label(proposal),
                        proposal_state(&proposal.id, &proposal.detail)
                    )
                })
                .collect()
        } else if let Some(choices) = &self.structured {
            choices
                .items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    format!("{} · {}", index + 1, structured_label(choices.kind, item))
                })
                .collect()
        } else {
            Vec::new()
        };
        if items.is_empty() || max_rows == 0 {
            return Vec::new();
        }
        let window = max_rows.min(items.len());
        let pick = self.pick.min(items.len() - 1);
        let start = pick.saturating_sub(window / 2).min(items.len() - window);
        items
            .into_iter()
            .enumerate()
            .skip(start)
            .take(window)
            .map(|(index, text)| {
                let marker = if index == pick { '>' } else { ' ' };
                format!("{marker} {text}")
            })
            .collect()
    }

    fn move_line(&self, down: bool) -> Option<usize> {
        if !self.input.contains('\n') {
            return None;
        }
        let cursor = self.cursor.min(self.input.len());
        let start = line_start(&self.input, cursor);
        let end = line_end(&self.input, cursor);
        let column = self.input[start..cursor].chars().count();
        if down {
            if end >= self.input.len() {
                return None;
            }
            let next = end + 1;
            let next_end = line_end(&self.input, next);
            return Some(next + byte_at_chars(&self.input[next..next_end], column));
        }
        if start == 0 {
            return None;
        }
        let previous_end = start - 1;
        let previous = line_start(&self.input, previous_end);
        Some(previous + byte_at_chars(&self.input[previous..previous_end], column))
    }

    fn fold_reasoning(&mut self) {
        if self.reasoning_chars == 0 && self.reasoning_text.is_empty() {
            return;
        }
        if self.reasoning_open {
            self.close_reasoning();
        }
        let chars = self.reasoning_chars;
        let text = std::mem::take(&mut self.reasoning_text);
        self.reasoning_chars = 0;
        if chars == 0 {
            return;
        }
        self.folded_reasoning = Some((chars, text));
        self.message(format!("Razonamiento plegado · {chars} caracteres"));
    }

    fn close_reasoning(&mut self) {
        self.reasoning_open = false;
        self.history
            .retain(|line| !line.starts_with(REASONING_MARK));
        self.history_bytes = self.history.iter().map(String::len).sum();
        self.history_revision = self.history_revision.wrapping_add(1);
    }

    fn toggle_reasoning(&mut self) {
        if self.reasoning_open {
            self.close_reasoning();
            return;
        }
        let text = if !self.reasoning_text.is_empty() {
            self.reasoning_text.clone()
        } else if let Some((_, text)) = &self.folded_reasoning {
            text.clone()
        } else {
            String::new()
        };
        if text.is_empty() {
            self.message("No hay razonamiento en esta respuesta.");
            return;
        }
        self.reasoning_open = true;
        self.message(format!("{REASONING_MARK}\n{}", clip_reasoning(&text)));
    }

    fn mark_grant(&mut self, id: &str, granted: bool) {
        let Some(proposal) = self.proposals.iter_mut().find(|item| item.id == id) else {
            return;
        };
        let (from_json, to_json, from_flag, to_flag) = if granted {
            (
                "\"granted\":false",
                "\"granted\":true",
                "concedida=false",
                "concedida=true",
            )
        } else {
            (
                "\"granted\":true",
                "\"granted\":false",
                "concedida=true",
                "concedida=false",
            )
        };
        proposal.detail = proposal
            .detail
            .replace(from_json, to_json)
            .replace(from_flag, to_flag);
    }

    fn page_rows(&self) -> usize {
        self.history_rows.max(1)
    }

    fn idle_status(&self) -> String {
        if self.partial_effects {
            if self.journal_hint.is_empty() {
                "efectos parciales · revisa el diario".into()
            } else {
                format!("efectos parciales · revisa {}", self.journal_hint)
            }
        } else if self.task_mode {
            "Preparado · Tarea".into()
        } else if self.ask_mode {
            "Preparado · Preguntar".into()
        } else {
            "Preparado · Editar".into()
        }
    }

    fn note_exclusive(&mut self, text: &str) {
        if text.contains("no concedida") || text.contains("servidor compartido") {
            self.exclusive = false;
        } else if text.contains("exclusividad concedida") {
            self.exclusive = true;
        }
    }

    fn apply_unloaded(&mut self, text: &str) {
        let Some(line) = text.lines().find(|line| line.contains("descargados:")) else {
            return;
        };
        let Some((_, names)) = line.split_once("descargados:") else {
            return;
        };
        for name in names.split(',') {
            let name = name.trim();
            if !name.is_empty() {
                self.session_models.retain(|item| item != name);
            }
        }
    }
}

fn liberation_status(text: &str) -> &'static str {
    if text.contains("descargados:") {
        "Modelos de esta sesión descargados"
    } else if text.contains("liberación de arranque no completada") {
        "Liberación de arranque incompleta"
    } else {
        "No se descargó ningún modelo"
    }
}

fn proposal_label(offer: &Offer) -> String {
    let parsed = serde_json::from_str::<Value>(&offer.detail).ok();
    let value = parsed.as_ref().unwrap_or(&Value::Null);
    let path = value["path"].as_str().unwrap_or("");
    if offer.id.starts_with("file:") {
        return match value["operation"].as_str() {
            Some("move") => format!(
                "Mover {path} → {}",
                value["destination"]
                    .as_str()
                    .unwrap_or("destino no disponible")
            ),
            Some("delete") => format!("Eliminar {path}"),
            _ => format!("Operación de archivo {path}"),
        };
    }
    if let Some(id) = offer.id.strip_prefix("command:") {
        return format!("Ejecutar prueba/comando {id}");
    }
    if let Some(id) = offer.id.strip_prefix("mcp:") {
        return format!("Servidor MCP {id}");
    }
    if offer.id == "exclusive" {
        return "Exclusividad del servidor Ollama".into();
    }
    if offer.id.starts_with("root:") {
        return "Raíz adicional propuesta · no aplicada".into();
    }
    offer.id.clone()
}

fn permission_detail(offer: &Offer) -> String {
    let mut lines = vec![
        proposal_label(offer),
        format!("Estado: {}", proposal_state(&offer.id, &offer.detail)),
        format!("Huella: {}", offer.fingerprint),
    ];
    match serde_json::from_str::<Value>(&offer.detail) {
        Ok(value) => lines.push(status_detail(&value)),
        Err(_) => lines.push(offer.detail.clone()),
    }
    lines.join("\n")
}

fn item_result(item: &Value) -> &Value {
    item.get("result").unwrap_or(item)
}

fn undo_fields(item: &Value) -> Option<(String, String, String, String)> {
    let value = item_result(item);
    if value["status"] != "verified" {
        return None;
    }
    let field = |name: &str| {
        value[name]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    Some((
        field("path")?,
        field("sha256")?,
        field("backup_path")?,
        field("backup_sha256")?,
    ))
}

fn choice_key(kind: ChoiceKind, item: &Value) -> String {
    match kind {
        ChoiceKind::Undo => undo_fields(item)
            .map(|fields| format!("{}\n{}\n{}\n{}", fields.0, fields.1, fields.2, fields.3))
            .unwrap_or_default(),
        ChoiceKind::Profiles => item["id"]
            .as_str()
            .or_else(|| item["name"].as_str())
            .unwrap_or("")
            .into(),
        _ => item["fingerprint"].as_str().unwrap_or("").into(),
    }
}

fn profile_label(value: &Value) -> String {
    let id = value["id"]
        .as_str()
        .or_else(|| value["name"].as_str())
        .or_else(|| value["profile_id"].as_str())
        .unwrap_or("perfil elegido");
    let parameters = value.get("profile").unwrap_or(value);
    if let (Some(context), Some(predict)) = (
        parameters["num_ctx"].as_u64(),
        parameters["num_predict"].as_u64(),
    ) {
        format!(
            "{id} · contexto {context} · generación {predict} · thinking {}",
            parameters["think"]
        )
    } else {
        id.into()
    }
}

fn recovery_review_label(item: &Value) -> &'static str {
    if item["needs_review"] != false {
        "pendiente"
    } else if item["reviewed"] == true {
        "revisada"
    } else {
        "sin revisión pendiente"
    }
}

fn recovery_progress(items: &[Value], blocked: bool) -> String {
    let pending = items
        .iter()
        .filter(|item| item["needs_review"] != false)
        .count();
    let reviewed = items
        .iter()
        .filter(|item| item["needs_review"] == false && item["reviewed"] == true)
        .count();
    if blocked && pending == 0 {
        return format!(
            "Recuperación: estado sin resolver · Revisadas: {reviewed} · Total: {}",
            items.len()
        );
    }
    format!(
        "Recuperación · Pendientes: {pending} · Revisadas: {reviewed} · Total: {}",
        items.len()
    )
}

fn structured_label(kind: ChoiceKind, item: &Value) -> String {
    match kind {
        ChoiceKind::Recovery => format!(
            "{} · {} · {} · {}",
            recovery_review_label(item),
            item["tool"].as_str().unwrap_or("operación"),
            item["path"].as_str().unwrap_or("diario/efectos externos"),
            item["state"].as_str().unwrap_or("estado no disponible")
        ),
        ChoiceKind::Profiles => profile_label(item),
        ChoiceKind::Archive => item["label"]
            .as_str()
            .or_else(|| item["path"].as_str())
            .unwrap_or("Evidencia revisada")
            .into(),
        ChoiceKind::Changes | ChoiceKind::Undo => {
            let value = item_result(item);
            format!(
                "{}{} · {}",
                if kind == ChoiceKind::Undo {
                    "Restaurar "
                } else {
                    ""
                },
                value["path"]
                    .as_str()
                    .unwrap_or_else(|| item["name"].as_str().unwrap_or("resultado observado")),
                value["status"].as_str().unwrap_or("sin estado")
            )
        }
    }
}

fn status_detail(value: &Value) -> String {
    fn label(key: &str) -> String {
        match key {
            "path" => "Ruta",
            "destination" => "Destino",
            "source_path" => "Origen completo",
            "program" => "Programa",
            "args" => "Argumentos",
            "cwd" => "Directorio de trabajo",
            "fingerprint" => "Huella vigente",
            "sha256" => "SHA observado",
            "base_sha256" => "SHA de base",
            "expected_sha256" => "SHA esperado",
            "backup_path" => "Respaldo",
            "backup_sha256" => "SHA del respaldo",
            "before" => "Antes",
            "after" => "Después",
            "diff" => "Cambios observados",
            "old" => "Coincidencia solicitada",
            "new" => "Reemplazo solicitado",
            "error" => "Motivo del fallo",
            "status" => "Estado",
            "arguments" => "Petición",
            "result" => "Resultado observado",
            "name" => "Nombre",
            "watched_files" => "Archivos vinculados a la concesión",
            "observed_files" => "Versiones observadas",
            "granted" => "Concedido",
            "entries" => "Entradas",
            "needs_review" => "Requiere revisión",
            "detail" => "Detalle",
            "reason" => "Motivo",
            "command" => "Comando observado",
            "context" => "Contexto",
            "requested" => "Solicitado",
            "observed" => "Observado",
            "loaded" => "Cargado",
            "selected" => "Seleccionado",
            "owned" => "De esta sesión",
            "limits" => "Límites",
            "usage" => "Uso",
            "profiles" => "Perfiles",
            "model" => "Modelo",
            "state" => "Estado de recuperación",
            "snapshot" => "Estado actual",
            "target" => "Resultado previsto",
            _ => return key.replace('_', " "),
        }
        .into()
    }
    fn append(value: &Value, depth: usize, lines: &mut Vec<String>) {
        let indent = "  ".repeat(depth);
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    if value.is_object() || value.is_array() {
                        lines.push(format!("{indent}{}:", label(key)));
                        append(value, depth + 1, lines);
                    } else {
                        lines.push(format!(
                            "{indent}{}: {}",
                            label(key),
                            value
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| value.to_string())
                        ));
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    if value.is_object() || value.is_array() {
                        lines.push(format!("{indent}—"));
                        append(value, depth + 1, lines);
                    } else {
                        lines.push(format!(
                            "{indent}• {}",
                            value
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| value.to_string())
                        ));
                    }
                }
            }
            Value::String(text) => lines.push(format!("{indent}{text}")),
            Value::Null => lines.push(format!("{indent}no disponible")),
            other => lines.push(format!("{indent}{other}")),
        }
    }
    let mut lines = Vec::new();
    append(value, 0, &mut lines);
    lines.join("\n")
}

fn proposal_state(id: &str, detail: &str) -> &'static str {
    if id.starts_with("root:") {
        "rechazada"
    } else if detail.contains("\"granted\":true") || detail.contains("concedida=true") {
        "concedido"
    } else {
        "no concedido"
    }
}

fn tool_extra(name: &str, arguments: &serde_json::Value, result: &serde_json::Value) -> String {
    let path = result
        .get("path")
        .and_then(serde_json::Value::as_str)
        .or_else(|| arguments.get("path").and_then(serde_json::Value::as_str))
        .unwrap_or("");
    if result["status"] != "verified"
        && let Some(error) = result["error"]
            .as_str()
            .or_else(|| result["reason"].as_str())
    {
        return if path.is_empty() {
            clip(error, 160)
        } else {
            format!("{path} · {}", clip(error, 160))
        };
    }
    if name == "move_file" && result["status"] == "verified" {
        return format!(
            "{path} → {}",
            result["destination"]
                .as_str()
                .unwrap_or("destino verificado")
        );
    }
    if name == "search_files" {
        let count = result
            .get("matches")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let label = if path.is_empty() { "búsqueda" } else { path };
        return format!("{label} · {count} coincidencias");
    }
    if name == "edit_file"
        && let (Some(old), Some(new)) = (
            arguments.get("old").and_then(serde_json::Value::as_str),
            arguments.get("new").and_then(serde_json::Value::as_str),
        )
    {
        let label = if path.is_empty() { "edición" } else { path };
        return format!("{label} · {} → {}", clip(old, 40), clip(new, 40));
    }
    if name == "list_files" {
        let count = result
            .get("entries")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let label = if path.is_empty() { "directorio" } else { path };
        return format!("{label} · {count} entradas");
    }
    if name == "run_command" {
        let id = arguments
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("comando");
        let code = result
            .get("exit_code")
            .and_then(serde_json::Value::as_u64)
            .or_else(|| {
                result
                    .pointer("/command/exit_code")
                    .and_then(serde_json::Value::as_u64)
            });
        if let Some(code) = code {
            return format!("{id} · salida {code}");
        }
        return id.to_owned();
    }
    if !path.is_empty() {
        return clip(path, 80);
    }
    result
        .get("error")
        .and_then(serde_json::Value::as_str)
        .map(|error| clip(error, 80))
        .unwrap_or_default()
}

fn clip(text: &str, limit: usize) -> String {
    text.chars()
        .take(limit)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn line_start(text: &str, cursor: usize) -> usize {
    let cursor = cursor.min(text.len());
    text[..cursor].rfind('\n').map_or(0, |index| index + 1)
}

fn line_end(text: &str, cursor: usize) -> usize {
    let cursor = cursor.min(text.len());
    text[cursor..]
        .find('\n')
        .map_or(text.len(), |index| cursor + index)
}

fn byte_at_chars(text: &str, count: usize) -> usize {
    text.char_indices()
        .nth(count)
        .map_or(text.len(), |(index, _)| index)
}

fn word_left(text: &str, cursor: usize) -> usize {
    let mut end = cursor.min(text.len());
    while end > 0 {
        let ch = text[..end].chars().next_back().unwrap();
        if !ch.is_whitespace() {
            break;
        }
        end -= ch.len_utf8();
    }
    while end > 0 {
        let ch = text[..end].chars().next_back().unwrap();
        if ch.is_whitespace() {
            break;
        }
        end -= ch.len_utf8();
    }
    end
}

fn is_extend(character: char) -> bool {
    matches!(character, '\u{200D}' | '\u{FE0F}' | '\u{FE0E}' | '\u{20E3}')
        || ('\u{1F3FB}'..='\u{1F3FF}').contains(&character)
        || ('\u{0300}'..='\u{036F}').contains(&character)
        || ('\u{1AB0}'..='\u{1AFF}').contains(&character)
        || ('\u{1DC0}'..='\u{1DFF}').contains(&character)
        || ('\u{20D0}'..='\u{20FF}').contains(&character)
        || ('\u{FE20}'..='\u{FE2F}').contains(&character)
}

fn is_regional(character: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&character)
}

pub(super) fn next_boundary(text: &str, start: usize) -> usize {
    let start = floor_char(text, start);
    let Some(first) = text[start..].chars().next() else {
        return start;
    };
    let mut index = start + first.len_utf8();
    if is_regional(first) {
        if let Some(second) = text[index..].chars().next()
            && is_regional(second)
        {
            index += second.len_utf8();
        }
        return index;
    }
    loop {
        let Some(character) = text[index..].chars().next() else {
            break;
        };
        if is_extend(character) {
            index += character.len_utf8();
            continue;
        }
        if text[..index].ends_with('\u{200D}') {
            index += character.len_utf8();
            continue;
        }
        break;
    }
    index
}

fn prev_boundary(text: &str, cursor: usize) -> usize {
    let cursor = floor_char(text, cursor);
    let mut index = 0;
    let mut previous = 0;
    while index < cursor {
        previous = index;
        let next = next_boundary(text, index);
        if next <= index {
            break;
        }
        index = next;
    }
    previous
}

fn floor_char(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn clip_reasoning(text: &str) -> String {
    let count = text.chars().count();
    if count <= REASONING_VIEW_CHARS {
        return text.to_owned();
    }
    let clipped: String = text.chars().take(REASONING_VIEW_CHARS).collect();
    format!("{clipped}\n… recortado; {count} caracteres en total")
}

fn word_right(text: &str, cursor: usize) -> usize {
    let mut index = cursor.min(text.len());
    while let Some(ch) = text[index..].chars().next() {
        if !ch.is_whitespace() {
            break;
        }
        index += ch.len_utf8();
    }
    while let Some(ch) = text[index..].chars().next() {
        if ch.is_whitespace() {
            break;
        }
        index += ch.len_utf8();
    }
    index
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ConsoleHistory {
    lines: Vec<String>,
    #[serde(default)]
    reasoning_chars: usize,
    #[serde(default)]
    reasoning: String,
}

fn size_label(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;
    if bytes >= GIB {
        let whole = bytes / GIB;
        let tenth = (bytes % GIB) * 10 / GIB;
        format!("{whole}.{tenth} GiB")
    } else {
        format!("{} MiB", bytes / MIB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_opens_menu_without_enter_and_escape_preserves_input() {
        let mut app = App::new(128, 16);
        app.key(Key::Text('/', 1));
        assert_eq!(app.matches().len(), COMMANDS.len());
        app.key(Key::Text('e', 1));
        assert_eq!(app.matches()[0].name, "/exit");
        app.key(Key::Escape);
        assert!(!app.menu_open());
        assert_eq!(app.input, "/e");
        assert!(!app.key(Key::Enter));
        assert!(app.history.back().unwrap().contains("no disponible"));
    }

    #[test]
    fn liberate_is_available_and_does_not_steal_the_exit_filter() {
        assert!(COMMANDS.iter().any(|command| command.name == "/liberate"));
        let mut app = App::new(128, 16);
        app.key(Key::Text('/', 1));
        app.key(Key::Text('l', 1));
        assert_eq!(app.matches().len(), 1);
        assert_eq!(app.matches()[0].name, "/liberate");
        assert!(!app.key(Key::Enter));
        assert!(matches!(app.take_action(), Some(Action::Liberate)));
    }

    #[test]
    fn selecting_a_command_executes_only_on_enter() {
        let mut app = App::new(128, 16);
        app.key(Key::Text('/', 1));
        assert!(!app.key(Key::Down));
        assert_eq!(app.matches()[app.selected].name, "/exit");
        assert!(app.key(Key::Enter));
    }

    #[test]
    fn unicode_editing_keeps_utf8_boundaries_and_input_limit() {
        let mut app = App::new(3, 2);
        for c in ['á', '🟢', 'z', 'x'] {
            app.key(Key::Text(c, 1));
        }
        assert_eq!(app.input, "á🟢z");
        app.key(Key::Left);
        app.key(Key::Backspace);
        assert_eq!(app.input, "áz");
        app.key(Key::Home);
        app.key(Key::Delete);
        assert_eq!(app.input, "z");
        app.message("uno");
        app.message("dos");
        app.message("tres");
        assert_eq!(app.history.len(), 2);
    }

    #[test]
    fn a_slash_in_regular_text_does_not_open_menu() {
        let mut app = App::new(128, 16);
        for c in "src/main.rs".chars() {
            app.key(Key::Text(c, 1));
        }
        assert!(!app.menu_open());
    }

    #[test]
    fn pasted_newlines_never_submit_chat_and_busy_preserves_new_input() {
        let mut app = App::new(128, 16);
        app.digest = "digest explícito".into();
        for line in ["primera línea", "segunda línea"] {
            for c in line.chars() {
                app.key(Key::Text(c, 1));
            }
            app.key(Key::Enter);
        }
        assert!(app.take_action().is_none());
        assert_eq!(app.input, "primera línea\nsegunda línea\n");
        app.key(Key::Submit);
        assert!(
            matches!(app.take_action(), Some(Action::Chat(text)) if text == "primera línea\nsegunda línea")
        );
        app.key(Key::Text('x', 1));
        app.key(Key::Submit);
        assert_eq!(app.input, "x");
        assert!(app.take_action().is_none());
        app.key(Key::Escape);
        assert!(matches!(app.take_action(), Some(Action::Cancel)));
    }

    #[test]
    fn slash_in_ordinary_request_allows_newline_and_explicit_submit() {
        let mut app = App::new(128, 16);
        app.digest = "digest explícito".into();
        for c in "/api/chat devuelve error".chars() {
            app.key(Key::Text(c, 1));
        }
        assert!(!app.menu_open());
        app.key(Key::Enter);
        assert_eq!(app.input, "/api/chat devuelve error\n");
        assert!(app.take_action().is_none());
        app.key(Key::Submit);
        assert!(
            matches!(app.take_action(), Some(Action::Chat(text)) if text == "/api/chat devuelve error")
        );
    }

    #[test]
    fn startup_notice_reports_the_result_without_a_working_status() {
        let mut app = App::new(128, 16);
        app.begin(Action::Discover, "Conectando con Ollama…");
        app.event(Event::Notice {
            role: "sistema".into(),
            text: "servidor compartido o exclusividad no concedida; no se descarga nada".into(),
        });
        assert!(app.busy);
        assert!(!app.status.contains("Trabajando"));
        assert_eq!(app.status, "No se descargó ningún modelo");
        assert!(!app.exclusive);
        app.key(Key::Escape);
        assert_eq!(app.status, "No se descargó ningún modelo");
        assert!(matches!(app.take_action(), Some(Action::Cancel)));
    }

    #[test]
    fn grants_and_liberation_use_their_own_status() {
        let mut app = App::new(128, 16);
        app.event(Event::Granted("mcp:eco".into()));
        assert_eq!(app.status, "Servidor MCP concedido");
        app.event(Event::Granted("command:test".into()));
        assert_eq!(app.status, "Comando concedido");
        app.event(Event::Granted("exclusive".into()));
        assert_eq!(app.status, "Exclusividad concedida");
        assert!(app.exclusive);
        app.session_models = vec!["propio".into(), "otro".into()];
        app.begin(Action::Liberate, "Comprobando memoria de esta sesión…");
        assert!(matches!(app.take_action(), Some(Action::Liberate)));
        app.event(Event::Liberated(
            "exclusividad concedida; se solicita la descarga\ndescargados: propio\nconservados: otro"
                .into(),
        ));
        assert!(!app.busy);
        assert_eq!(app.status, "Modelos de esta sesión descargados");
        assert_eq!(app.session_models, vec!["otro".to_string()]);
        app.event(Event::Liberated(
            "servidor compartido o exclusividad no concedida; no se descarga nada".into(),
        ));
        assert!(!app.exclusive);
        assert_eq!(app.status, "No se descargó ningún modelo");
        app.key(Key::Escape);
        assert_eq!(app.status, "No se descargó ningún modelo");
        assert!(app.take_action().is_none());
    }

    #[test]
    fn task_mode_is_visible_and_partial_effects_survive_model_selection() {
        let mut app = App::new(128, 16);
        for character in "/task".chars() {
            app.key(Key::Text(character, 1));
        }
        assert!(!app.key(Key::Enter));
        assert!(app.status.contains("Tarea"));
        app.event(Event::Tool {
            role: "implementador".into(),
            call_id: "1".into(),
            name: "edit_file".into(),
            arguments: serde_json::json!({"path":"calc.py","old":"a - b","new":"a + b"}),
            result: serde_json::json!({"status":"partial","path":"calc.py","error":"ventana"}),
            journal: String::new(),
        });
        assert!(app.partial_effects);
        assert_eq!(app.status, "efectos parciales · revisa el diario");
        let tool_line = app.history.back().unwrap();
        assert!(tool_line.contains("edit_file"));
        assert!(tool_line.contains("partial"));
        assert!(tool_line.contains("ventana"));
        assert!(!tool_line.contains("a - b → a + b"));
        assert_eq!(app.details.back().unwrap()["arguments"]["old"], "a - b");
        assert_eq!(app.details.back().unwrap()["result"]["status"], "partial");
        assert!(!tool_line.contains('{'));
        app.event(Event::Selected(crate::providers::ollama::SelectedModel {
            model: crate::providers::ollama::Model {
                name: "qwen".into(),
                digest: "abc".into(),
                size: 1,
            },
            capabilities: vec!["tools".into()],
            context_limit: 4096,
        }));
        assert!(app.partial_effects);
        assert_eq!(app.status, "efectos parciales · revisa el diario");
        assert_eq!(app.session_models, vec!["qwen".to_string()]);
    }

    #[test]
    fn permissions_show_a_short_row_until_enter_asks_for_detail() {
        let mut app = App::new(256, 16);
        app.event(Event::Permissions(vec![Offer {
            id: "mcp:eco".into(),
            fingerprint: "abc".into(),
            detail: r#"{"granted":false,"program":"python"}"#.into(),
        }]));
        let row = app
            .history
            .iter()
            .find(|line| line.contains("Servidor MCP eco"))
            .unwrap();
        assert!(row.contains("no concedido"));
        assert!(!row.contains("program"));
        app.key(Key::Text('1', 1));
        assert!(!app.key(Key::Enter));
        assert!(app.take_action().is_none());
        assert!(app.granting);
        let detail = app.detail_view.as_ref().unwrap();
        assert!(detail.title.contains("Servidor MCP eco"));
        assert!(detail.text.contains("Programa: python"));
        assert!(detail.text.contains("Huella: abc"));
        assert!(app.footer().contains("confirmar"));
        app.key(Key::Submit);
        assert!(matches!(app.take_action(), Some(Action::Grant { id, .. }) if id == "mcp:eco"));
    }

    #[test]
    fn page_up_moves_more_than_one_line_and_failures_name_the_cause() {
        let mut app = App::new(128, 128);
        app.view_rows = 40;
        for index in 0..80 {
            app.message(format!("línea {index}"));
        }
        let mut config: crate::config::Config =
            serde_json::from_str(include_str!("../../config/defaults.json")).unwrap();
        config.ui.compact_header = true;
        crate::ui::render::frame(
            &mut app,
            std::path::Path::new("proyecto"),
            &config.ui,
            None,
            80,
            40,
        );
        let visible = app.history_rows;
        assert!(visible > 1);
        app.key(Key::PageUp);
        assert_eq!(app.scroll, visible);
        app.key(Key::PageDown);
        assert_eq!(app.scroll, 0);
        app.digest = "digest".into();
        app.event(Event::Failed(
            "historial demasiado grande para el contexto configurado".into(),
        ));
        assert!(app.status.contains("Contexto agotado"));
        app.event(Event::Failed("el servidor rechazó la petición".into()));
        assert_eq!(app.status, "Operación fallida · puedes volver a intentarlo");
        app.digest.clear();
        app.event(Event::Failed("conexión rechazada".into()));
        assert!(app.status.contains("/model para consultar o elegir"));
    }

    #[test]
    fn model_list_shows_size_and_whether_it_is_loaded() {
        let mut app = App::new(128, 16);
        app.event(Event::Models(vec![
            ListedModel {
                name: "qwen".into(),
                size: 2 * 1024 * 1024 * 1024,
                loaded: Some(true),
            },
            ListedModel {
                name: "tiny".into(),
                size: 50 * 1024 * 1024,
                loaded: Some(false),
            },
        ]));
        let text = app.history.iter().cloned().collect::<Vec<_>>().join("\n");
        assert!(text.contains("qwen"));
        assert!(text.contains("2.0 GiB"));
        assert!(text.contains("en memoria"));
        assert!(text.contains("tiny"));
        assert!(text.contains("50 MiB"));
        assert!(text.contains("no cargado"));
    }

    #[test]
    fn tool_activity_and_budget_do_not_end_the_operation() {
        let mut app = App::new(128, 16);
        app.begin(Action::Chat("hola".into()), "Generando");
        app.event(Event::Activity {
            role: "explorador".into(),
            text: "leyendo calc.py".into(),
        });
        assert!(app.busy);
        assert_eq!(app.status, "explorador · leyendo calc.py");
        app.event(Event::Budget {
            used: 100,
            limit: 4096,
        });
        assert_eq!(app.context_used, Some(100));
        assert_eq!(app.context_limit, Some(4096));
        assert!(app.busy);
        app.event(Event::Tool {
            role: "explorador".into(),
            call_id: "9".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({"path":"calc.py"}),
            result: serde_json::json!({"status":"verified","path":"calc.py","sha256":"abcd"}),
            journal: String::new(),
        });
        let line = app.history.back().unwrap();
        assert!(line.contains("read_file"));
        assert!(line.contains("verified"));
        assert!(line.contains("calc.py"));
        assert!(!line.contains("sha256"));
        assert!(app.busy);
    }

    #[test]
    fn reasoning_stays_folded_and_a_task_step_names_the_role() {
        let mut app = App::new(128, 16);
        app.busy = true;
        app.event(Event::Reasoning {
            chars: 800,
            text: "a".repeat(800),
        });
        assert!(app.busy);
        assert!(app.status.contains("razonando"));
        assert!(app.status.contains("800"));
        app.event(Event::Delta("ok".into()));
        assert!(app.busy);
        assert!(app.status.starts_with("Generando"));
        assert!(
            app.history
                .iter()
                .any(|line| line.contains("Razonamiento plegado") && line.contains("800"))
        );
        app.event(Event::TaskStep {
            role: "tester".into(),
            step: 3,
        });
        assert!(app.busy);
        assert_eq!(app.task_step, Some(("tester".into(), 3)));
        for character in "/chat".chars() {
            app.key(Key::Text(character, 1));
        }
        assert!(!app.key(Key::Enter));
        assert!(app.busy);
        assert!(app.task_step.is_some());
        app.busy = false;
        for character in "/chat".chars() {
            app.key(Key::Text(character, 1));
        }
        assert!(!app.key(Key::Enter));
        assert!(app.task_step.is_none());
        assert!(!app.task_mode);
    }

    #[test]
    fn arrows_select_a_model_and_revoke_or_refuse_a_permission() {
        let mut app = App::new(256, 16);
        app.event(Event::Models(vec![
            ListedModel {
                name: "uno".into(),
                size: 1024 * 1024,
                loaded: Some(false),
            },
            ListedModel {
                name: "dos".into(),
                size: 1024 * 1024,
                loaded: Some(true),
            },
        ]));
        assert!(!app.key(Key::Down));
        app.key(Key::Submit);
        assert!(matches!(app.take_action(), Some(Action::Select(name)) if name == "dos"));
        app.busy = false;
        app.event(Event::Permissions(vec![
            Offer {
                id: "command:eco".into(),
                fingerprint: "abc".into(),
                detail: r#"{"granted":true}"#.into(),
            },
            Offer {
                id: "root:0".into(),
                fingerprint: "rechazada".into(),
                detail: "raíz extra".into(),
            },
        ]));
        app.key(Key::Submit);
        assert!(matches!(app.take_action(), Some(Action::Revoke { id }) if id == "command:eco"));
        app.event(Event::Revoked("command:eco".into()));
        assert!(app.granting);
        assert!(app.proposals[0].detail.contains("\"granted\":false"));
        assert!(!app.key(Key::Down));
        app.key(Key::Submit);
        assert!(app.take_action().is_none());
        assert!(
            app.history
                .back()
                .unwrap()
                .contains("no se concede ni se retira")
        );
    }

    #[test]
    fn search_names_matches_and_partial_effects_keep_the_journal() {
        let mut app = App::new(256, 16);
        app.event(Event::Tool {
            role: "explorador".into(),
            call_id: "2".into(),
            name: "search_files".into(),
            arguments: serde_json::json!({"path":"calc.py","needle":"a"}),
            result: serde_json::json!({
                "status": "verified",
                "path": "calc.py",
                "matches": [{}, {}, {}]
            }),
            journal: ".pillcli/sessions/1.jsonl".into(),
        });
        assert!(
            app.history
                .back()
                .unwrap()
                .contains("calc.py · 3 coincidencias")
        );
        app.event(Event::Tool {
            role: "implementador".into(),
            call_id: "3".into(),
            name: "edit_file".into(),
            arguments: serde_json::json!({"path":"calc.py"}),
            result: serde_json::json!({"status":"partial","path":"calc.py","error":"ventana"}),
            journal: ".pillcli/sessions/1.jsonl".into(),
        });
        assert!(app.partial_effects);
        assert!(app.status.contains(".pillcli/sessions/1.jsonl"));
        app.event(Event::Selected(crate::providers::ollama::SelectedModel {
            model: crate::providers::ollama::Model {
                name: "qwen".into(),
                digest: "abc".into(),
                size: 1,
            },
            capabilities: vec!["tools".into()],
            context_limit: 4096,
        }));
        assert!(app.partial_effects);
        assert!(app.status.contains(".pillcli/sessions/1.jsonl"));
    }

    #[test]
    fn typing_keeps_the_page_and_motion_stays_inside_the_line() {
        let mut app = App::new(128, 16);
        app.scroll = 5;
        app.key(Key::Text('a', 1));
        assert_eq!(app.scroll, 5);
        app.input = "uno\ndos".into();
        app.cursor = app.input.len();
        app.key(Key::Home);
        assert_eq!(app.cursor, 4);
        app.key(Key::End);
        assert_eq!(app.cursor, app.input.len());
        app.key(Key::Up);
        assert_eq!(app.cursor, 3);
        app.input = "hola mundo".into();
        app.cursor = app.input.len();
        app.key(Key::WordLeft);
        assert_eq!(app.cursor, 5);
        app.key(Key::WordRight);
        assert_eq!(app.cursor, app.input.len());
        app.digest = "digest".into();
        app.input = "hola".into();
        app.cursor = app.input.len();
        app.scroll = 4;
        app.key(Key::Submit);
        assert_eq!(app.scroll, 0);
        assert!(matches!(app.take_action(), Some(Action::Chat(text)) if text == "hola"));
        app.key(Key::Up);
        assert_eq!(app.input, "hola");
    }

    #[test]
    fn liberate_asks_before_unloading_session_models() {
        let mut app = App::new(128, 32);
        app.exclusive = true;
        app.session_models = vec!["qwen".into(), "otro".into()];
        for character in "/liberate".chars() {
            app.key(Key::Text(character, 1));
        }
        assert!(!app.key(Key::Enter));
        assert!(app.take_action().is_none());
        let line = app.history.back().unwrap().clone();
        assert!(line.contains("qwen"));
        assert!(line.contains("otro"));
        app.key(Key::Escape);
        assert!(app.take_action().is_none());
        assert!(app.history.back().unwrap().contains("cancelada"));
        for character in "/liberate".chars() {
            app.key(Key::Text(character, 1));
        }
        assert!(!app.key(Key::Enter));
        app.key(Key::Enter);
        assert!(app.take_action().is_none(), "un salto pegado no descarga");
        assert!(app.confirm_liberate);
        app.key(Key::Submit);
        assert!(matches!(app.take_action(), Some(Action::Liberate)));
    }

    #[test]
    fn ctrl_r_opens_and_closes_reasoning_without_mixing_it_into_the_answer() {
        let mut app = App::new(128, 32);
        app.key(Key::ToggleReasoning);
        assert!(app.history.back().unwrap().contains("No hay razonamiento"));
        app.busy = true;
        app.event(Event::Reasoning {
            chars: 4,
            text: "hola".into(),
        });
        app.key(Key::ToggleReasoning);
        assert!(app.history.back().unwrap().contains("hola"));
        assert!(!app.response.contains("hola"));
        app.key(Key::ToggleReasoning);
        assert!(!app.history.iter().any(|line| line.contains("hola")));
        app.event(Event::Delta("ok".into()));
        assert_eq!(app.response, "ok");
        assert!(
            app.history
                .iter()
                .any(|line| line.contains("Razonamiento plegado") && line.contains("4"))
        );
        app.key(Key::ToggleReasoning);
        assert!(app.history.iter().any(|line| line.contains("hola")));
        app.key(Key::ToggleReasoning);
        assert!(!app.history.iter().any(|line| line.contains("hola")));
    }

    #[test]
    fn journal_command_is_before_liberate_and_reads_without_clearing_partial_effects() {
        let mut app = App::new(128, 32);
        app.key(Key::Text('/', 1));
        app.key(Key::Text('j', 1));
        assert_eq!(app.matches().len(), 1);
        assert_eq!(app.matches()[0].name, "/journal");
        assert!(!app.key(Key::Enter));
        assert!(matches!(app.take_action(), Some(Action::Journal)));
        app.busy = false;
        app.partial_effects = true;
        app.journal_hint = ".pillcli/sessions/1.jsonl".into();
        app.event(Event::Journal(
            "Diario 1.jsonl\nexplorer · observed · read_file · verified · a.py".into(),
        ));
        assert!(!app.busy);
        assert!(app.partial_effects);
        assert!(app.status.contains(".pillcli"));
        assert!(app.history.back().unwrap().contains("read_file"));
        app.event(Event::Restored { dropped: 2 });
        app.event(Event::Selected(crate::providers::ollama::SelectedModel {
            model: crate::providers::ollama::Model {
                name: "qwen".into(),
                digest: "abc".into(),
                size: 1,
            },
            capabilities: vec!["tools".into()],
            context_limit: 4096,
        }));
        assert!(app.partial_effects);
        assert!(
            app.history
                .iter()
                .any(|line| line.contains("Conversación recuperada") && line.contains("2"))
        );
    }

    #[test]
    fn backspace_removes_a_whole_grapheme() {
        let mut app = App::new(64, 8);
        let family = "👨\u{200D}👩\u{200D}👧";
        app.input = format!("{family}z");
        app.cursor = app.input.len();
        app.key(Key::Backspace);
        assert_eq!(app.input, family);
        app.key(Key::Backspace);
        assert!(app.input.is_empty());
        let marked = "e\u{0301}";
        app.input = format!("a{marked}");
        app.cursor = app.input.len();
        app.key(Key::Backspace);
        assert_eq!(app.input, "a");
        app.input = "❤\u{FE0F}z".to_string();
        app.cursor = 0;
        app.key(Key::Delete);
        assert_eq!(app.input, "z");
        app.key(Key::Left);
        assert_eq!(app.cursor, 0);
        let flag = "🇪🇸";
        app.input = format!("{flag}!");
        app.cursor = app.input.len();
        app.key(Key::Left);
        app.key(Key::Backspace);
        assert_eq!(app.input, "!");
    }

    #[test]
    fn escape_closes_models_and_preserves_effective_model_and_draft() {
        let mut app = App::new(256, 32);
        app.model_label = "modelo vigente".into();
        app.digest = "digest vigente".into();
        app.input = "borrador sin enviar".into();
        app.cursor = app.input.len();
        app.event(Event::Models(vec![ListedModel {
            name: "otro".into(),
            size: 1,
            loaded: Some(false),
        }]));
        assert!(app.choosing);
        assert!(app.footer().contains("Ctrl+Enter elegir"));
        app.key(Key::Escape);
        assert!(!app.choosing);
        assert_eq!(app.model_label, "modelo vigente");
        assert_eq!(app.digest, "digest vigente");
        assert_eq!(app.input, "borrador sin enviar");
        assert!(app.take_action().is_none());
    }

    #[test]
    fn file_permissions_use_semantic_rows_and_keep_full_fingerprint_in_detail() {
        let mut app = App::new(256, 32);
        let fingerprint = "a".repeat(64);
        app.event(Event::Permissions(vec![Offer{id:format!("file:{fingerprint}"),fingerprint:fingerprint.clone(),detail:json!({"operation":"move","path":"origen.txt","destination":"destino.txt","granted":false}).to_string()}]));
        let row = app.choice_rows(1).remove(0);
        assert!(row.contains("Mover origen.txt → destino.txt"));
        assert!(!row.contains(&fingerprint));
        assert!(app.footer().contains("Enter detalle"));
        assert!(app.footer().contains("conceder/retirar"));
        assert!(!app.footer().contains("Enter línea"));
        app.key(Key::Enter);
        assert!(app.take_action().is_none());
        assert!(
            app.detail_view
                .as_ref()
                .unwrap()
                .text
                .contains(&fingerprint)
        );
    }

    #[test]
    fn ask_mode_dispatches_readonly_action_and_edit_restores_explicit_editing() {
        let mut app = App::new(256, 32);
        app.digest = "digest explícito".into();
        for character in "/ask".chars() {
            app.key(Key::Text(character, 1));
        }
        app.key(Key::Enter);
        assert!(app.ask_mode && !app.task_mode);
        assert!(app.mode_label().contains("solo lectura"));
        app.input = "lee este proyecto".into();
        app.cursor = app.input.len();
        app.key(Key::Submit);
        assert!(matches!(app.take_action(),Some(Action::Ask(text)) if text=="lee este proyecto"));
        app.busy = false;
        for character in "/edit".chars() {
            app.key(Key::Text(character, 1));
        }
        app.key(Key::Enter);
        assert!(!app.ask_mode && !app.task_mode);
        app.input = "edita este proyecto".into();
        app.cursor = app.input.len();
        app.key(Key::Submit);
        assert!(
            matches!(app.take_action(),Some(Action::Chat(text)) if text=="edita este proyecto")
        );
    }

    #[test]
    fn recovery_needs_preview_and_exact_current_fingerprint_before_human_ack() {
        let mut app = App::new(256, 32);
        let entry = json!({"tool":"mcp__servidor__write","path":"dato.txt","state":"unknown","needs_review":true,"fingerprint":"evidencia-1"});
        app.event(Event::Recovery {
            report: json!({"needs_review":true,"entries":[entry]}),
        });
        app.key(Key::Submit);
        assert!(app.detail_view.is_some());
        assert!(app.take_action().is_none());
        app.key(Key::Submit);
        assert!(
            matches!(app.take_action(),Some(Action::AcknowledgeRecovery{fingerprint}) if fingerprint=="evidencia-1")
        );
        assert!(app.partial_effects);
        app.event(Event::RecoveryReviewed{report:json!({"needs_review":false,"entries":[{"tool":"mcp__servidor__write","state":"unknown","needs_review":false,"reviewed":true,"fingerprint":"evidencia-1"}]})});
        assert!(!app.partial_effects);
        assert_eq!(app.recovery_report["entries"][0]["state"], "unknown");
    }

    #[test]
    fn screen_byte_quota_preserves_utf8_and_reports_retention_without_affecting_copy() {
        let mut config: crate::config::Config =
            serde_json::from_str(include_str!("../../config/defaults.json")).unwrap();
        config.ui.max_history_bytes = 4096;
        let mut app = App::new(256, 64);
        app.configure_ui(&config.ui);
        app.message("antiguo".repeat(300));
        app.message("á".repeat(3000));
        assert!(app.history_bytes <= config.ui.max_history_bytes);
        assert!(
            app.history
                .iter()
                .all(|text| std::str::from_utf8(text.as_bytes()).is_ok())
        );
        assert!(app.history_omitted > 0);
        assert!(app.history_clipped_bytes > 0);
        app.last_answer = "respuesta íntegra\nsegunda línea".into();
        app.key(Key::Copy);
        assert_eq!(
            app.copy_requested.as_deref(),
            Some("respuesta íntegra\nsegunda línea")
        );
    }
    #[test]
    fn recovery_progress_keeps_unknown_history_and_counts_unreviewed_evidence_conservatively() {
        let mut app = App::new(4096, 128);
        let entries = json!([
            {"tool":"mcp__s__write","state":"unknown","needs_review":false,"reviewed":true,"fingerprint":"a"},
            {"tool":"edit_file","state":"unknown","needs_review":true,"reviewed":true,"fingerprint":"b"},
            {"tool":"create_file","state":"applied","needs_review":false,"reviewed":false,"fingerprint":"c"},
            {"tool":"diario","state":"unknown","fingerprint":"d"}]);
        app.event(Event::Recovery {
            report: json!({"needs_review":true,"entries":entries}),
        });
        assert_eq!(
            app.recovery_heading(),
            Some("Recuperación · Pendientes: 2 · Revisadas: 1 · Total: 4")
        );
        let rows = app.choice_rows(10);
        assert!(rows[0].contains("revisada") && rows[0].contains("unknown"));
        assert!(rows[1].contains("pendiente"));
        assert!(rows[2].contains("sin revisión pendiente"));
        assert!(rows[3].contains("pendiente"));
        app.key(Key::Enter);
        app.key(Key::Submit);
        assert!(app.take_action().is_none());
        assert!(app.partial_effects);
        app.event(Event::Recovery {
            report: json!({"needs_review":true,"entries":[]}),
        });
        assert!(
            app.recovery_heading()
                .unwrap()
                .contains("estado sin resolver")
        );
        assert!(!app.recovery_heading().unwrap().contains("0 pendientes"));
    }

    #[test]
    fn profile_detail_announces_existing_apply_action_after_preview() {
        let mut app = App::new(4096, 128);
        app.event(Event::Profiles(vec![
            json!({"name":"codigo","num_ctx":8192,"num_predict":1024,"think":false}),
        ]));
        app.key(Key::Enter);
        assert!(app.footer().contains("Ctrl+Enter aplicar perfil"));
        app.key(Key::Submit);
        assert!(matches!(app.take_action(),Some(Action::SelectProfile(name)) if name=="codigo"));
    }
}
