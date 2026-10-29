//! Reconciliación de solo lectura. Nunca reproduce una herramienta del diario.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::files::{FileError, Files};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Applied,
    Pending,
    Unknown,
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub journal: String,
    pub call_id: String,
    pub tool: String,
    pub path: Option<String>,
    pub destination: Option<String>,
    pub backup_path: Option<String>,
    pub state: State,
    pub needs_review: bool,
    pub detail: String,
    pub expected_sha256: Option<String>,
    pub intended_sha256: Option<String>,
    pub current: Value,
    pub destination_current: Value,
    pub journal_sha256: String,
    pub fingerprint: String,
    pub reviewed: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub entries: Vec<Entry>,
    pub needs_review: bool,
}

impl Report {
    fn push(&mut self, entry: Entry) {
        self.needs_review |= entry.needs_review;
        self.entries.push(entry);
    }

    pub fn summary(&self) -> String {
        let count = |state| {
            self.entries
                .iter()
                .filter(|entry| entry.state == state)
                .count()
        };
        let mut text = format!(
            "Reconciliación local: {} aplicadas, {} pendientes, {} desconocidas. No se repitieron operaciones.",
            count(State::Applied),
            count(State::Pending),
            count(State::Unknown)
        );
        if self.needs_review {
            // No vuelca rutas/texto de un diario potencialmente dañado en la consola.
            // El detalle estructurado se conserva en `entries` para inspección.
            text.push_str(" Efectos bloqueados: revisa .pillcli/sessions y los respaldos.");
        }
        text
    }
}

/// Los límites proceden de la configuración validada del proyecto.
pub fn reconcile(
    files: &Files,
    max_sessions: usize,
    max_journal_bytes: usize,
    max_record_bytes: usize,
) -> Report {
    let mut report = Report::default();
    if max_sessions == 0 || max_journal_bytes == 0 || max_record_bytes == 0 {
        report.push(invalid(
            ".pillcli/sessions",
            "límites de recuperación inválidos",
        ));
        return report;
    }
    let journals = match files.session_journals(max_sessions, max_journal_bytes) {
        Ok(journals) => journals,
        Err(error) => {
            report.push(invalid(".pillcli/sessions", &error.to_string()));
            return report;
        }
    };
    let mut hashes = BTreeMap::new();
    for (name, bytes) in journals {
        hashes.insert(name.clone(), digest(&bytes));
        scan(files, &name, &bytes, max_record_bytes, &mut report);
    }
    let reviews = match load_reviews(files) {
        Ok(value) => value,
        Err(error) => {
            report.push(invalid(
                "recovery.json",
                &format!("registro de revisión no válido: {error}"),
            ));
            Reviews::default()
        }
    };
    let mut indices = std::collections::BTreeMap::<String, usize>::new();
    for entry in &mut report.entries {
        let index = *indices.entry(entry.journal.clone()).or_default();
        *indices.get_mut(&entry.journal).expect("índice insertado") += 1;
        entry.journal_sha256 = hashes.get(&entry.journal).cloned().unwrap_or_default();
        entry.current = entry
            .path
            .as_deref()
            .map(|path| view_snapshot(files, path))
            .unwrap_or(Value::Null);
        entry.destination_current = entry
            .destination
            .as_deref()
            .map(|path| view_snapshot(files, path))
            .unwrap_or(Value::Null);
        let evidence =
            json!({"version":1,"journal_sha256":entry.journal_sha256,"index":index,"entry":entry});
        entry.fingerprint = digest(evidence.to_string().as_bytes());
        if entry.needs_review && reviews.fingerprints.contains(&entry.fingerprint) {
            entry.reviewed = true;
            entry.needs_review = false;
            entry.detail.push_str("; revisión humana explícita de esta evidencia, sin afirmar éxito de la herramienta");
        }
    }
    report.needs_review = report.entries.iter().any(|entry| entry.needs_review);
    report
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn view_snapshot(files: &Files, path: &str) -> Value {
    match files.inspect_operation(path) {
        Ok(value) => value,
        Err(FileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            json!({"path":path,"absent":true})
        }
        Err(error) => json!({"path":path,"unavailable":error.to_string()}),
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Reviews {
    version: u32,
    fingerprints: BTreeSet<String>,
}

fn load_reviews(files: &Files) -> Result<Reviews, String> {
    match files
        .read_private_checked("recovery.json")
        .map_err(|e| e.to_string())?
        .bytes
    {
        None => Ok(Reviews {
            version: 1,
            ..Reviews::default()
        }),
        Some(bytes) => {
            let value: Reviews = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if value.version != 1
                || value
                    .fingerprints
                    .iter()
                    .any(|f| f.len() != 64 || !f.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err("versión o huella de revisión inválida".into());
            }
            Ok(value)
        }
    }
}

/// Solo acciones humanas: nunca registrar como herramienta de un modelo.
pub fn acknowledge(
    files: &Files,
    fingerprint: &str,
    max_sessions: usize,
    max_journal_bytes: usize,
    max_record_bytes: usize,
) -> Result<Report, String> {
    let report = reconcile(files, max_sessions, max_journal_bytes, max_record_bytes);
    let entry = report
        .entries
        .iter()
        .find(|entry| entry.fingerprint == fingerprint)
        .ok_or("la evidencia cambió; vuelve a mostrar recuperación")?;
    if entry.journal_sha256.is_empty() {
        return Err("no se puede reconocer una enumeración o registro privado incompleto; corrige la causa y vuelve a inspeccionar".into());
    }
    if entry.reviewed {
        return Ok(report);
    }
    if !entry.needs_review {
        return Err("esta entrada no necesita reconocimiento".into());
    }
    let mut reviews = load_reviews(files)?;
    reviews.fingerprints.insert(fingerprint.into());
    let bytes = serde_json::to_vec(&reviews).map_err(|e| e.to_string())?;
    if bytes.len() > max_journal_bytes {
        return Err(
            "revisiones superan agents.max_journal_bytes; archiva sesiones revisadas".into(),
        );
    }
    files
        .write_private("recovery.json", &bytes)
        .map_err(|e| e.to_string())?;
    Ok(reconcile(
        files,
        max_sessions,
        max_journal_bytes,
        max_record_bytes,
    ))
}

fn valid_private(name: &str, bytes: &[u8]) -> bool {
    match name {
        "model.json" => crate::session::valid_transcript(bytes),
        "console.json" => serde_json::from_slice::<Value>(bytes).is_ok_and(|value| {
            value["lines"]
                .as_array()
                .is_some_and(|lines| lines.iter().all(Value::is_string))
        }),
        "recovery.json" => serde_json::from_slice::<Reviews>(bytes).is_ok_and(|r| {
            r.version == 1
                && r.fingerprints
                    .iter()
                    .all(|f| f.len() == 64 && f.bytes().all(|b| b.is_ascii_hexdigit()))
        }),
        _ => false,
    }
}

pub fn archive_offers(
    files: &Files,
    max_sessions: usize,
    max_journal_bytes: usize,
    max_record_bytes: usize,
) -> Result<Value, String> {
    let report = reconcile(files, max_sessions, max_journal_bytes, max_record_bytes);
    let journals = files
        .session_journals(max_sessions, max_journal_bytes)
        .map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    for (name, bytes) in journals {
        if report
            .entries
            .iter()
            .any(|entry| entry.journal == name && entry.needs_review)
        {
            continue;
        }
        let mut item = json!({"kind":"session","journal":name,"sha256":digest(&bytes),"bytes":bytes.len(),"operation":"archive_session"});
        item["fingerprint"] = json!(digest(item.to_string().as_bytes()));
        items.push(item);
    }
    let project = files
        .project_archive_offers()
        .unwrap_or_else(|error| json!({"items":[],"truncated":true,"error":error.to_string()}));
    if !report.needs_review {
        items.extend(
            files
                .private_archive_offers_with(valid_private)
                .map_err(|e| e.to_string())?,
        );
        items.extend(
            project["items"]
                .as_array()
                .ok_or("ofertas de respaldo no validas")?
                .iter()
                .cloned(),
        );
    }
    Ok(
        json!({"items":items,"project_scan":project,"retention":files.retention_status().unwrap_or_else(|error|json!({"unknown":true,"error":error.to_string()})),"needs_review":report.needs_review,"summary":"Archivo con huella confirmada; conserva bytes en .pillcli/archive. Versiones bloqueadas mientras haya efectos sin revisar."}),
    )
}

pub fn confirm_archive(
    files: &Files,
    fingerprint: &str,
    max_sessions: usize,
    max_journal_bytes: usize,
    max_record_bytes: usize,
) -> Result<Value, String> {
    let offers = archive_offers(files, max_sessions, max_journal_bytes, max_record_bytes)?;
    let item = offers["items"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["fingerprint"].as_str() == Some(fingerprint))
        })
        .ok_or("la propuesta de archivo cambió; vuelve a mostrarla")?;
    let archived = if item["kind"] == "private_version" {
        files.archive_private_version_with(fingerprint, valid_private)
    } else if item["kind"] == "project_transaction" {
        files.archive_project_transaction(fingerprint)
    } else {
        files.archive_session(
            item["journal"].as_str().ok_or("diario no válido")?,
            item["sha256"].as_str().ok_or("SHA no válido")?,
        )
    }
    .map_err(|e| e.to_string())?;
    let mut result = archive_offers(files, max_sessions, max_journal_bytes, max_record_bytes)?;
    result["status"] = json!("verified");
    result["archived"] = archived;
    Ok(result)
}

struct Call {
    requested: Value,
    observed: Option<Value>,
    ambiguous: bool,
}

fn scan(files: &Files, name: &str, bytes: &[u8], max_record: usize, report: &mut Report) {
    let mut calls = Vec::<Call>::new();
    let mut indices = BTreeMap::<String, usize>::new();
    let mut session = None::<String>;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        report.push(invalid(
            name,
            "último registro truncado o sin terminador durable",
        ));
    }
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        if line.len().saturating_add(1) > max_record {
            report.push(invalid(name, "registro supera agents.max_record_bytes"));
            continue;
        }
        let value = match serde_json::from_slice::<Value>(line) {
            Ok(value) if value.is_object() => value,
            _ => {
                report.push(invalid(
                    name,
                    "registro JSON inválido; efectos previos desconocidos",
                ));
                continue;
            }
        };
        let Some(current_session) = field(&value, "session") else {
            report.push(invalid(name, "registro sin identificador de sesión"));
            continue;
        };
        if session
            .as_deref()
            .is_some_and(|previous| previous != current_session)
        {
            report.push(invalid(name, "identificadores de sesión contradictorios"));
            continue;
        }
        session.get_or_insert_with(|| current_session.to_owned());
        let stage = field(&value, "stage").unwrap_or("");
        if !matches!(stage, "requested" | "observed") {
            if !matches!(
                stage,
                "agent_started" | "agent_completed" | "agent_completed_from_tool_evidence"
            ) || field(&value, "tool").is_some_and(has_effect)
            {
                report.push(invalid(
                    name,
                    "etapa del diario inválida o incompatible con efectos",
                ));
            }
            continue;
        }
        let Some(tool) = field(&value, "tool") else {
            report.push(invalid(name, "llamada sin herramienta"));
            continue;
        };
        if !has_effect(tool) {
            continue;
        }
        let Some(id) = field(&value, "call_id") else {
            report.push(invalid(name, "llamada con efectos sin identificador"));
            continue;
        };
        if stage == "requested" {
            if let Some(index) = indices.get(id) {
                calls[*index].ambiguous = true;
            } else {
                indices.insert(id.to_owned(), calls.len());
                calls.push(Call {
                    requested: value,
                    observed: None,
                    ambiguous: false,
                });
            }
        } else if let Some(index) = indices.get(id) {
            let call = &mut calls[*index];
            if call.observed.is_some() || field(&call.requested, "tool") != Some(tool) {
                call.ambiguous = true;
            }
            call.observed = Some(value);
        } else {
            report.push(invalid(
                name,
                "resultado con efectos sin intención correlacionada",
            ));
        }
    }
    for call in calls {
        report.push(reconcile_call(files, name, call));
    }
}

fn has_effect(tool: &str) -> bool {
    matches!(
        tool,
        "create_file"
            | "edit_file"
            | "edit_file_many"
            | "restore_backup"
            | "move_file"
            | "delete_file"
            | "run_command"
    ) || tool.starts_with("mcp__")
}

fn field<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn invalid(journal: &str, detail: &str) -> Entry {
    Entry {
        journal: journal.to_owned(),
        call_id: String::new(),
        tool: "diario".into(),
        path: None,
        destination: None,
        backup_path: None,
        state: State::Unknown,
        needs_review: true,
        detail: detail.to_owned(),
        expected_sha256: None,
        intended_sha256: None,
        current: Value::Null,
        destination_current: Value::Null,
        journal_sha256: String::new(),
        fingerprint: String::new(),
        reviewed: false,
    }
}

enum Snapshot {
    Missing,
    Hash(String),
    Unavailable(String),
}

fn snapshot(files: &Files, path: &str) -> Snapshot {
    match files.inspect_operation(path) {
        Ok(value) => match field(&value, "sha256") {
            Some(hash) => Snapshot::Hash(hash.to_owned()),
            None => Snapshot::Unavailable("inspección sin SHA-256".into()),
        },
        Err(FileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Snapshot::Missing
        }
        Err(error) => Snapshot::Unavailable(error.to_string()),
    }
}

fn matches_hash(snapshot: &Snapshot, expected: Option<&str>) -> bool {
    match (snapshot, expected) {
        (Snapshot::Hash(actual), Some(expected)) => actual.eq_ignore_ascii_case(expected),
        _ => false,
    }
}

fn reconcile_call(files: &Files, name: &str, call: Call) -> Entry {
    let requested = &call.requested;
    let observed = call.observed.as_ref();
    let tool = field(requested, "tool").unwrap_or("");
    let path = field(requested, "path");
    let destination = field(requested, "destination")
        .or_else(|| observed.and_then(|value| field(value, "destination")));
    let mut entry = Entry {
        journal: name.to_owned(),
        call_id: field(requested, "call_id").unwrap_or("").to_owned(),
        tool: tool.to_owned(),
        path: path.map(str::to_owned),
        destination: destination.map(str::to_owned),
        backup_path: observed
            .and_then(|value| field(value, "backup_path"))
            .map(str::to_owned),
        state: State::Unknown,
        needs_review: true,
        detail: "no se puede demostrar el resultado; no se repite la operación".into(),
        expected_sha256: field(requested, "expected_sha256").map(str::to_owned),
        intended_sha256: field(requested, "target_sha256").map(str::to_owned),
        current: Value::Null,
        destination_current: Value::Null,
        journal_sha256: String::new(),
        fingerprint: String::new(),
        reviewed: false,
    };
    if call.ambiguous {
        entry.detail = "intenciones o resultados duplicados/contradictorios".into();
        return entry;
    }
    if let Some(value) = observed {
        match field(value, "status") {
            Some("failed" | "cancelled")
                if (tool.starts_with("mcp__") && value["effect_not_started"] != true)
                    || (tool == "run_command"
                        && (value["command"]["timed_out"] == true
                            || value["command"]["cancelled"] == true)) =>
            {
                entry.detail =
                    "fallo histórico sin prueba de ausencia de efectos; requiere revisión".into();
            }
            Some("verified") => {
                entry.state = State::Applied;
                entry.needs_review = false;
                entry.detail = "resultado verificado registrado antes del reinicio".into();
                if let (Some(path), Some(expected)) = (path, field(value, "sha256"))
                    && matches!(
                        tool,
                        "create_file" | "edit_file" | "edit_file_many" | "restore_backup"
                    )
                    && !matches_hash(&snapshot(files, path), Some(expected))
                {
                    entry
                        .detail
                        .push_str("; la versión actual difiere o no está disponible");
                }
            }
            Some("failed" | "cancelled" | "awaiting_confirmation") => {
                entry.state = State::Pending;
                entry.needs_review = false;
                entry.detail =
                    "fallo, cancelación o falta de confirmación registrados; no se reintenta automáticamente".into();
            }
            Some("partial" | "unknown" | "applied") => {
                entry.detail = "efectos con verificación incompleta registrados; requieren revisión incluso tras reiniciar".into();
            }
            _ => entry.detail = "resultado sin estado final reconocido".into(),
        }
        return entry;
    }
    if tool == "run_command" || tool.starts_with("mcp__") {
        entry.detail =
            "llamada con efectos externos sin resultado durable; el disco no demuestra su cierre"
                .into();
        return entry;
    }
    let Some(path) = path else {
        entry.detail = "intención de archivo sin ruta".into();
        return entry;
    };
    let current = snapshot(files, path);
    if let Snapshot::Unavailable(error) = &current {
        entry.detail = format!("no se pudo inspeccionar la ruta: {error}");
        return entry;
    }
    let base = field(requested, "expected_sha256");
    let target = field(requested, "target_sha256");
    let state = match tool {
        "create_file" if matches!(current, Snapshot::Missing) => Some(State::Pending),
        "create_file" if matches_hash(&current, target) => Some(State::Applied),
        "edit_file" | "edit_file_many" | "restore_backup" if matches_hash(&current, target) => {
            Some(State::Applied)
        }
        "edit_file" | "edit_file_many" | "restore_backup" if matches_hash(&current, base) => {
            Some(State::Pending)
        }
        "delete_file"
            if matches!(current, Snapshot::Missing)
                && base.is_some_and(|hash| {
                    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                }) =>
        {
            Some(State::Applied)
        }
        "delete_file" if matches_hash(&current, base) => Some(State::Pending),
        "move_file" => destination.and_then(|destination| {
            let target = snapshot(files, destination);
            if matches!(current, Snapshot::Missing) && matches_hash(&target, base) {
                Some(State::Applied)
            } else if matches_hash(&current, base) && matches!(target, Snapshot::Missing) {
                Some(State::Pending)
            } else {
                None
            }
        }),
        _ => None,
    };
    if let Some(state) = state {
        entry.state = state;
        entry.needs_review = false;
        entry.detail = match state {
            State::Applied => "estado previsto presente en disco; la llamada no se repite",
            State::Pending => "precondición intacta o creación ausente; la llamada no se repite",
            State::Unknown => unreachable!(),
        }
        .into();
    }
    entry
}
