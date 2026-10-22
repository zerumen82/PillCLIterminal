//! API local de Ollama. Nunca descarga modelos, sigue redirecciones ni usa proxies.
use std::{
    future::Future,
    net::IpAddr,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::OllamaConfig;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Ollama: respuesta final vacía")]
    EmptyResponse,
    #[error("Ollama: {0}")]
    Protocol(String),
    #[error("Conexión Ollama: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Respuesta Ollama no válida: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Model {
    pub name: String,
    pub digest: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SelectedModel {
    pub model: Model,
    pub capabilities: Vec<String>,
    pub context_limit: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thinking: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub function: ToolFunction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub arguments: Value,
}

/// Bytes reservados antes de pedir generación: mensajes, herramientas, formato, tope y margen.
pub fn reserved_context(
    messages: &[Message],
    tools: &[Value],
    format: Option<&Value>,
    num_predict: u32,
) -> Result<usize, Error> {
    let messages = messages
        .iter()
        .map(|message| serde_json::to_vec(message).map(|bytes| bytes.len().saturating_add(64)))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .fold(0usize, usize::saturating_add);
    Ok(messages
        .saturating_add(serde_json::to_vec(tools)?.len())
        .saturating_add(serde_json::to_vec(&format)?.len())
        .saturating_add(num_predict as usize)
        .saturating_add(128))
}

impl Message {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_name: None,
            thinking: String::new(),
        }
    }
}

#[derive(Debug)]
pub enum StreamUpdate {
    Text(String),
    Thinking { chars: usize, text: String },
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub content: String,
    pub elapsed_ms: u128,
    pub first_token_ms: Option<u128>,
    pub metrics: Value,
    pub tool_calls: Vec<ToolCall>,
    pub thinking: String,
}

#[derive(Clone)]
pub struct Ollama {
    client: Client,
    base: Url,
    config: OllamaConfig,
}

/// E2 solo permite HTTP local. Rechazar también credenciales, rutas y consultas.
pub fn local_url(value: &str) -> Result<Url, Error> {
    let url = Url::parse(value).map_err(|e| Error::Protocol(e.to_string()))?;
    let local = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "http"
        || !local
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Protocol(
            "ollama.url debe ser HTTP en loopback, sin credenciales, ruta ni consulta".into(),
        ));
    }
    Ok(url)
}

impl Ollama {
    pub fn new(config: &OllamaConfig) -> Result<Self, Error> {
        let base = local_url(&config.url)?;
        let mut builder = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_millis(config.connect_timeout_ms))
            .timeout(Duration::from_millis(config.response_timeout_ms));
        if base.host_str() == Some("localhost") {
            builder = builder.resolve(
                "localhost",
                std::net::SocketAddr::from((
                    [127, 0, 0, 1],
                    base.port_or_known_default().unwrap_or(80),
                )),
            );
        }
        let client = builder.build()?;
        Ok(Self {
            client,
            base,
            config: config.clone(),
        })
    }

    fn endpoint(&self, path: &str) -> Url {
        self.base.join(path).expect("rutas API constantes válidas")
    }

    async fn body(&self, response: reqwest::Response) -> Result<Value, Error> {
        let status = response.status();
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len().saturating_add(chunk.len()) > self.config.max_metadata_bytes {
                return Err(Error::Protocol(
                    "metadatos superan ollama.max_metadata_bytes".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "HTTP {}: {}",
                status.as_u16(),
                String::from_utf8_lossy(&bytes)
            )));
        }
        let value: Value = serde_json::from_slice(&bytes)?;
        check_error(&value)?;
        Ok(value)
    }

    pub async fn models(&self) -> Result<Vec<Model>, Error> {
        #[derive(Deserialize)]
        struct Tags {
            models: Vec<Model>,
        }
        let value = self
            .body(self.client.get(self.endpoint("api/tags")).send().await?)
            .await?;
        let mut models = serde_json::from_value::<Tags>(value)?.models;
        if models
            .iter()
            .any(|m| m.name.is_empty() || m.digest.is_empty())
        {
            return Err(Error::Protocol("modelo sin nombre o digest".into()));
        }
        models.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(models)
    }

    pub async fn select(&self, name: &str) -> Result<SelectedModel, Error> {
        let before = self
            .models()
            .await?
            .into_iter()
            .find(|m| m.name == name)
            .ok_or_else(|| {
                Error::Protocol(format!("modelo no instalado: {name}; no se descargará"))
            })?;
        let value = self
            .body(
                self.client
                    .post(self.endpoint("api/show"))
                    .json(&json!({"model":name}))
                    .send()
                    .await?,
            )
            .await?;
        let capabilities: Vec<String> = serde_json::from_value(
            value
                .get("capabilities")
                .cloned()
                .ok_or_else(|| Error::Protocol("faltan capacidades en /api/show".into()))?,
        )?;
        if !capabilities.iter().any(|c| c == "completion") {
            return Err(Error::Protocol(
                "el modelo no anuncia generación de texto".into(),
            ));
        }
        let info = value
            .get("model_info")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::Protocol("falta model_info".into()))?;
        let architecture = info
            .get("general.architecture")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("falta arquitectura del modelo".into()))?;
        let context_limit = info
            .get(&format!("{architecture}.context_length"))
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::Protocol("contexto máximo desconocido".into()))?;
        if u64::from(self.config.num_ctx) > context_limit {
            return Err(Error::Protocol(format!(
                "ollama.num_ctx={} supera el contexto anunciado {context_limit}",
                self.config.num_ctx
            )));
        }
        let after = self.models().await?.into_iter().find(|m| m.name == name);
        if after.as_ref().map(|m| &m.digest) != Some(&before.digest) {
            return Err(Error::Protocol(
                "el modelo cambió durante la selección; vuelve a elegirlo".into(),
            ));
        }
        Ok(SelectedModel {
            model: before,
            capabilities,
            context_limit,
        })
    }

    pub async fn running(&self) -> Result<Value, Error> {
        self.body(self.client.get(self.endpoint("api/ps")).send().await?)
            .await
    }

    /// Pide a Ollama que descargue un modelo. No borra el modelo del disco.
    pub async fn unload(&self, name: &str) -> Result<(), Error> {
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err(Error::Protocol("modelo a descargar inválido".into()));
        }
        self.body(
            self.client
                .post(self.endpoint("api/generate"))
                .json(&json!({"model": name, "keep_alive": 0, "stream": false}))
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }

    pub async fn chat<F, Fut>(
        &self,
        model: &SelectedModel,
        messages: &[Message],
        delta: F,
    ) -> Result<Outcome, Error>
    where
        F: FnMut(StreamUpdate) -> Fut,
        Fut: Future<Output = Result<(), Error>>,
    {
        self.chat_tools(model, messages, &[], delta).await
    }

    pub async fn chat_tools<F, Fut>(
        &self,
        model: &SelectedModel,
        messages: &[Message],
        tools: &[Value],
        delta: F,
    ) -> Result<Outcome, Error>
    where
        F: FnMut(StreamUpdate) -> Fut,
        Fut: Future<Output = Result<(), Error>>,
    {
        self.chat_formatted(model, messages, tools, None, delta)
            .await
    }

    pub async fn chat_formatted<F, Fut>(
        &self,
        model: &SelectedModel,
        messages: &[Message],
        tools: &[Value],
        format: Option<&Value>,
        mut delta: F,
    ) -> Result<Outcome, Error>
    where
        F: FnMut(StreamUpdate) -> Fut,
        Fut: Future<Output = Result<(), Error>>,
    {
        // La misma reserva que muestra la cabecera. No es un contador del tokenizer.
        let estimate = reserved_context(messages, tools, format, self.config.num_predict)?;
        if estimate > self.config.num_ctx as usize {
            return Err(Error::Protocol("historial demasiado grande para el contexto configurado; vuelve a seleccionar el modelo para iniciar otra conversación".into()));
        }
        // Detectar reemplazos antes de cada generación; no adoptar otro digest en silencio.
        if self
            .models()
            .await?
            .iter()
            .find(|m| m.name == model.model.name)
            .map(|m| &m.digest)
            != Some(&model.model.digest)
        {
            return Err(Error::Protocol(
                "el modelo seleccionado cambió o desapareció; selecciónalo de nuevo".into(),
            ));
        }
        let start = Instant::now();
        let mut request = json!({
            "model":model.model.name,"messages":messages,"stream":true,
            "options":{"num_ctx":self.config.num_ctx,"num_predict":self.config.num_predict,"temperature":self.config.temperature}
        });
        if let Some(format) = format {
            request["format"] = format.clone();
        }
        if !tools.is_empty() {
            if !model.capabilities.iter().any(|c| c == "tools") {
                return Err(Error::Protocol("el modelo no anuncia herramientas".into()));
            }
            request["tools"] = json!(tools);
        }
        if model.capabilities.iter().any(|c| c == "thinking") {
            request["think"] = json!(self.config.think);
        }
        // No enviar keep_alive: el servidor es compartido y E2 no descarga recursos.
        let response = self
            .client
            .post(self.endpoint("api/chat"))
            .json(&request)
            .send()
            .await?;
        if !response.status().is_success() {
            self.body(response).await?;
            return Err(Error::Protocol("respuesta HTTP inesperada".into()));
        }
        let mut stream = response.bytes_stream();
        let mut parser = Ndjson::new(self.config.max_line_bytes, self.config.max_response_bytes);
        let mut state = StreamState::default();
        while let Some(chunk) = stream.next().await {
            for value in parser.push(&chunk?)? {
                if let Some(outcome) = self
                    .confirmed(
                        model,
                        accept(value, &mut state, start, &mut delta, !tools.is_empty()).await,
                    )
                    .await?
                {
                    return Ok(outcome);
                }
            }
        }
        if let Some(value) = parser.finish()?
            && let Some(outcome) = self
                .confirmed(
                    model,
                    accept(value, &mut state, start, &mut delta, !tools.is_empty()).await,
                )
                .await?
        {
            return Ok(outcome);
        }
        Err(Error::Protocol(
            "flujo interrumpido sin done=true; respuesta parcial no confirmada".into(),
        ))
    }

    async fn confirmed(
        &self,
        model: &SelectedModel,
        result: Result<Option<Outcome>, Error>,
    ) -> Result<Option<Outcome>, Error> {
        if matches!(&result, Ok(Some(_)) | Err(Error::EmptyResponse)) {
            self.verify_digest(model).await?;
        }
        result
    }

    async fn verify_digest(&self, selected: &SelectedModel) -> Result<(), Error> {
        if self
            .models()
            .await?
            .iter()
            .find(|m| m.name == selected.model.name)
            .map(|m| &m.digest)
            != Some(&selected.model.digest)
        {
            return Err(Error::Protocol(
                "el digest cambió durante la generación; respuesta no confirmada".into(),
            ));
        }
        Ok(())
    }
}

fn check_error(value: &Value) -> Result<(), Error> {
    if let Some(error) = value.get("error") {
        return Err(Error::Protocol(format!("error del servidor: {error}")));
    }
    Ok(())
}

#[derive(Default)]
struct StreamState {
    content: String,
    first: Option<u128>,
    calls: ToolFragments,
    thinking: String,
    thinking_notified: usize,
}

async fn accept<F, Fut>(
    value: Value,
    state: &mut StreamState,
    start: Instant,
    delta: &mut F,
    tools_allowed: bool,
) -> Result<Option<Outcome>, Error>
where
    F: FnMut(StreamUpdate) -> Fut,
    Fut: Future<Output = Result<(), Error>>,
{
    let StreamState {
        content,
        first,
        calls,
        thinking,
        thinking_notified,
    } = state;
    check_error(&value)?;
    let done = value
        .get("done")
        .and_then(Value::as_bool)
        .ok_or_else(|| Error::Protocol("fragmento sin done booleano".into()))?;
    let message = value
        .get("message")
        .ok_or_else(|| Error::Protocol("fragmento sin message".into()))?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return Err(Error::Protocol("rol inesperado en respuesta".into()));
    }
    if let Some(calls) = message.get("tool_calls") {
        let calls = calls
            .as_array()
            .ok_or_else(|| Error::Protocol("tool_calls no es un array".into()))?;
        if !calls.is_empty() && !tools_allowed {
            return Err(Error::Protocol(
                "el modelo solicitó herramientas; todavía no están disponibles".into(),
            ));
        }
    }
    calls.push(message)?;
    if let Some(value) = message.get("thinking") {
        thinking.push_str(
            value
                .as_str()
                .ok_or_else(|| Error::Protocol("thinking no es texto".into()))?,
        );
        let chars = thinking.chars().count();
        if chars > 0 && (*thinking_notified == 0 || chars - *thinking_notified >= 64) {
            *thinking_notified = chars;
            delta(StreamUpdate::Thinking {
                chars,
                text: thinking.clone(),
            })
            .await?;
        }
    }
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("message.content no es texto".into()))?;
    if !text.is_empty() {
        first.get_or_insert_with(|| start.elapsed().as_millis());
        content.push_str(text);
        delta(StreamUpdate::Text(text.to_owned())).await?;
    }
    if !done {
        return Ok(None);
    }
    if value.get("done_reason").and_then(Value::as_str) == Some("length") {
        return Err(Error::Protocol(
            "límite de generación alcanzado; respuesta parcial no confirmada".into(),
        ));
    }
    if let Some(reason) = value.get("done_reason")
        && reason.as_str() != Some("stop")
    {
        return Err(Error::Protocol(format!(
            "finalización no confirmada: {reason}"
        )));
    }
    let tool_calls = calls.finish()?;
    if content.trim().is_empty() && tool_calls.is_empty() {
        return Err(Error::EmptyResponse);
    }
    let mut metrics = value;
    metrics.as_object_mut().unwrap().remove("message");
    Ok(Some(Outcome {
        content: content.clone(),
        elapsed_ms: start.elapsed().as_millis(),
        first_token_ms: *first,
        metrics,
        tool_calls,
        thinking: thinking.clone(),
    }))
}

/// Fragmentos sin efectos: solo se convierten en llamadas tras done válido.
#[derive(Default)]
struct ToolFragments(std::collections::BTreeMap<u64, Value>);

impl ToolFragments {
    fn push(&mut self, message: &Value) -> Result<(), Error> {
        let Some(calls) = message.get("tool_calls") else {
            return Ok(());
        };
        for call in calls
            .as_array()
            .ok_or_else(|| Error::Protocol("tool_calls no es array".into()))?
        {
            let function = call
                .get("function")
                .and_then(Value::as_object)
                .ok_or_else(|| Error::Protocol("llamada sin function".into()))?;
            let index = function.get("index").or_else(|| call.get("index"));
            let index = match index {
                Some(value) => value
                    .as_u64()
                    .ok_or_else(|| Error::Protocol("índice de llamada inválido".into()))?,
                None => self.0.len() as u64,
            };
            let current = self
                .0
                .entry(index)
                .or_insert_with(|| json!({"name":"","arguments":null}));
            if let Some(name) = function.get("name") {
                let name = name
                    .as_str()
                    .ok_or_else(|| Error::Protocol("nombre de herramienta inválido".into()))?;
                let old = current["name"].as_str().unwrap();
                if old != name {
                    current["name"] = json!(format!("{old}{name}"));
                }
            }
            if let Some(arguments) = function.get("arguments") {
                if arguments.is_string() {
                    let previous = current["arguments"].as_str().unwrap_or("");
                    current["arguments"] =
                        json!(format!("{previous}{}", arguments.as_str().unwrap()));
                } else if arguments.is_object() {
                    if current["arguments"].is_null() {
                        current["arguments"] = json!({});
                    }
                    let object = current["arguments"].as_object_mut().ok_or_else(|| {
                        Error::Protocol("mezcla de fragmentos de argumentos".into())
                    })?;
                    for (key, value) in arguments.as_object().unwrap() {
                        if object.insert(key.clone(), value.clone()).is_some() {
                            return Err(Error::Protocol("argumento duplicado ambiguo".into()));
                        }
                    }
                } else {
                    return Err(Error::Protocol(
                        "argumentos de herramienta inválidos".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn finish(&self) -> Result<Vec<ToolCall>, Error> {
        self.0
            .values()
            .map(|function| {
                let name = function["name"].as_str().unwrap_or("");
                if name.is_empty() {
                    return Err(Error::Protocol("llamada incompleta sin nombre".into()));
                }
                let args = &function["arguments"];
                let arguments: Value = if let Some(text) = args.as_str() {
                    serde_json::from_str(text)?
                } else {
                    args.clone()
                };
                if !arguments.is_object() {
                    return Err(Error::Protocol(
                        "llamada incompleta: argumentos no son objeto".into(),
                    ));
                }
                Ok(ToolCall {
                    function: ToolFunction {
                        name: name.into(),
                        arguments,
                    },
                })
            })
            .collect()
    }
}

/// Acumula bytes, no texto: un carácter UTF-8 puede cruzar paquetes TCP.
pub struct Ndjson {
    pending: Vec<u8>,
    total: usize,
    max_line: usize,
    max_total: usize,
}

impl Ndjson {
    pub fn new(max_line: usize, max_total: usize) -> Self {
        Self {
            pending: Vec::new(),
            total: 0,
            max_line,
            max_total,
        }
    }
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Value>, Error> {
        self.total = self.total.saturating_add(chunk.len());
        if self.total > self.max_total {
            return Err(Error::Protocol(
                "respuesta supera ollama.max_response_bytes".into(),
            ));
        }
        let mut values = Vec::new();
        for &byte in chunk {
            if byte == b'\n' {
                if !self.pending.iter().all(u8::is_ascii_whitespace) {
                    values.push(serde_json::from_slice(&self.pending)?);
                }
                self.pending.clear();
            } else {
                self.pending.push(byte);
                if self.pending.len() > self.max_line {
                    return Err(Error::Protocol("línea supera ollama.max_line_bytes".into()));
                }
            }
        }
        Ok(values)
    }
    pub fn finish(self) -> Result<Option<Value>, Error> {
        if self.pending.iter().all(u8::is_ascii_whitespace) {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&self.pending)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reserved_context_of_empty_messages_matches_the_gate() {
        let tools = Vec::<Value>::new();
        let format: Option<&Value> = None;
        let used = reserved_context(&[], &tools, format, 512).unwrap();
        let expected = serde_json::to_vec(&tools).unwrap().len()
            + serde_json::to_vec(&format).unwrap().len()
            + 512
            + 128;
        assert_eq!(used, expected);
    }

    #[test]
    fn local_endpoint_rejects_remote_and_credential_leaks() {
        for url in [
            "https://localhost:11434",
            "http://example.com",
            "http://user@localhost",
            "http://localhost/api",
            "http://localhost/?a=b",
        ] {
            assert!(local_url(url).is_err(), "{url}");
        }
        for url in [
            "http://127.0.0.1:11434",
            "http://localhost:11434",
            "http://[::1]:11434",
        ] {
            assert!(local_url(url).is_ok(), "{url}");
        }
    }
    #[test]
    fn ndjson_handles_split_utf8_crlf_limits_and_truncation() {
        let bytes = "{\"text\":\"á🟢\"}\r\n{\"done\":true}".as_bytes();
        let mut parser = Ndjson::new(100, 200);
        let mut result = Vec::new();
        for byte in bytes {
            result.extend(parser.push(&[*byte]).unwrap());
        }
        result.push(parser.finish().unwrap().unwrap());
        assert_eq!(result[0]["text"], "á🟢");
        assert_eq!(result[1]["done"], true);
        assert!(Ndjson::new(2, 10).push(b"123").is_err());
        assert!(Ndjson::new(10, 2).push(b"123").is_err());
        let mut parser = Ndjson::new(10, 20);
        parser.push(b"{\"done\":").unwrap();
        assert!(parser.finish().is_err());
    }

    #[tokio::test]
    async fn tool_fragments_stay_pending_until_valid_final_and_reject_ambiguous_arguments() {
        let mut state = StreamState::default();
        let start = Instant::now();
        let mut delta = |_| std::future::ready(Ok(()));
        let pending = accept(json!({"done":false,"message":{"role":"assistant","content":"","tool_calls":[{"function":{"index":0,"name":"read_file","arguments":"{\"path\":"}}]}}), &mut state, start, &mut delta, true).await.unwrap();
        assert!(pending.is_none());
        assert!(state.calls.finish().is_err());
        let completed = accept(json!({"done":true,"done_reason":"stop","message":{"role":"assistant","content":"","tool_calls":[{"function":{"index":0,"arguments":"\"calc.py\"}"}}]}}), &mut state, start, &mut delta, true).await.unwrap().unwrap();
        assert_eq!(completed.tool_calls.len(), 1);
        assert_eq!(
            completed.tool_calls[0].function.arguments["path"],
            "calc.py"
        );
        let mut calls = ToolFragments::default();
        calls.push(&json!({"tool_calls":[{"function":{"index":0,"name":"edit_file","arguments":{"path":"a"}}}]})).unwrap();
        assert!(
            calls
                .push(&json!({"tool_calls":[{"function":{"index":0,"arguments":{"path":"b"}}}]}))
                .is_err()
        );
    }

    #[tokio::test]
    async fn malformed_final_records_errors_and_length_cannot_confirm_partial_text() {
        for record in [
            json!({"done":true,"message":{"role":"assistant"}}),
            json!({"done":true,"message":{"role":"assistant","content":12}}),
            json!({"done":true,"message":{"role":"assistant","content":"","tool_calls":{}}}),
            json!({"done":true,"message":{"role":"tool","content":"x"}}),
            json!({"done":true,"done_reason":"length","message":{"role":"assistant","content":""}}),
            json!({"done":true,"done_reason":7,"message":{"role":"assistant","content":""}}),
            json!({"done":true,"message":{"role":"assistant","content":"","tool_calls":[{}]}}),
            json!({"error":"fallo explícito"}),
        ] {
            let mut state = StreamState {
                content: "fragmento previo".into(),
                ..StreamState::default()
            };
            assert!(
                accept(
                    record,
                    &mut state,
                    Instant::now(),
                    &mut |_| { std::future::ready(Ok(())) },
                    false
                )
                .await
                .is_err()
            );
        }
    }

    #[tokio::test]
    async fn thinking_notifies_by_blocks_and_stays_out_of_the_answer() {
        let mut state = StreamState::default();
        let seen = std::cell::RefCell::new(Vec::new());
        let mut delta = |update| {
            seen.borrow_mut().push(match update {
                StreamUpdate::Thinking { chars, .. } => chars,
                StreamUpdate::Text(text) => text.len(),
            });
            std::future::ready(Ok(()))
        };
        let thinking = "a".repeat(70);
        let pending = accept(
            json!({"done":false,"message":{"role":"assistant","content":"","thinking":thinking}}),
            &mut state,
            Instant::now(),
            &mut delta,
            false,
        )
        .await
        .unwrap();
        assert!(pending.is_none());
        assert_eq!(seen.borrow().as_slice(), &[70]);
        let done = accept(
            json!({"done":true,"done_reason":"stop","message":{"role":"assistant","content":"listo","thinking":"b"}}),
            &mut state,
            Instant::now(),
            &mut delta,
            false,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(done.content, "listo");
        assert_eq!(done.thinking.chars().count(), 71);
        assert_eq!(seen.borrow().as_slice(), &[70, 5]);
    }
}
