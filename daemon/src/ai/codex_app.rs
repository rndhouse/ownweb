use super::AiOpinion;
use crate::core::ContentItem;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fmt, path::Path, process::Stdio, time::Duration};
use tokio::{
    net::TcpStream,
    process::{Child, Command},
    sync::Mutex,
    time::{sleep, timeout},
};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:39177";
const DEFAULT_MODEL: &str = "gpt-5.3-codex-spark";
const DEFAULT_EFFORT: &str = "low";
const DEFAULT_TIMEOUT_MS: u64 = 8000;

/// Codex app-server analyzer backed by one local app-server process.
pub struct CodexAppAnalyzer {
    config: CodexAppConfig,
    state: Mutex<CodexAppState>,
}

impl CodexAppAnalyzer {
    /// Builds a Codex app-server analyzer from environment variables.
    pub fn from_env() -> Self {
        let config = CodexAppConfig::from_env();
        Self {
            config,
            state: Mutex::new(CodexAppState::default()),
        }
    }

    /// Gets short Codex opinions for X posts.
    pub async fn x_opinions(&self, items: &[ContentItem]) -> Result<Vec<AiOpinion>, CodexAppError> {
        let prompt_items = prompt_items(items);
        if prompt_items.is_empty() {
            return Ok(Vec::new());
        }

        match timeout(
            self.config.request_timeout,
            self.run_x_opinion_turn(prompt_items),
        )
        .await
        {
            Ok(Ok(opinions)) => Ok(opinions),
            Ok(Err(error)) => {
                self.reset_session().await;
                Err(error)
            }
            Err(_elapsed) => {
                self.reset_session().await;
                Err(CodexAppError::Timeout)
            }
        }
    }

    async fn run_x_opinion_turn(
        &self,
        prompt_items: Vec<PromptItem>,
    ) -> Result<Vec<AiOpinion>, CodexAppError> {
        let mut state = self.state.lock().await;
        self.check_child_status(&mut state).await?;
        self.ensure_session_started(&mut state).await?;

        let thread_id = state
            .thread_id
            .clone()
            .ok_or_else(|| CodexAppError::Protocol("missing reusable Codex thread".into()))?;
        let session = state
            .session
            .as_mut()
            .ok_or_else(|| CodexAppError::Protocol("missing Codex app-server session".into()))?;
        let result = session
            .start_turn(&self.config, &thread_id, prompt_items)
            .await;

        if result.is_err() {
            state.session = None;
            state.thread_id = None;
        }

        result
    }

    async fn ensure_session_started(&self, state: &mut CodexAppState) -> Result<(), CodexAppError> {
        if state.session.is_some() && state.thread_id.is_some() {
            return Ok(());
        }

        match self.try_connect_session().await {
            Ok(mut session) => {
                session.initialize().await?;
                let thread_id = session.start_thread(&self.config).await?;
                state.session = Some(session);
                state.thread_id = Some(thread_id);
                return Ok(());
            }
            Err(_error) => {}
        }

        self.ensure_server_started(state).await?;

        for _ in 0..30 {
            match self.try_connect_session().await {
                Ok(mut session) => {
                    session.initialize().await?;
                    let thread_id = session.start_thread(&self.config).await?;
                    state.session = Some(session);
                    state.thread_id = Some(thread_id);
                    return Ok(());
                }
                Err(_error) => sleep(Duration::from_millis(100)).await,
            }
        }

        let mut session = self.try_connect_session().await?;
        session.initialize().await?;
        let thread_id = session.start_thread(&self.config).await?;
        state.session = Some(session);
        state.thread_id = Some(thread_id);
        Ok(())
    }

    async fn try_connect_session(&self) -> Result<CodexAppSession, CodexAppError> {
        let (socket, _response) = connect_async(&self.config.ws_url).await?;
        Ok(CodexAppSession {
            socket,
            next_request_id: 1,
            cwd: self.config.cwd.clone(),
        })
    }

    async fn check_child_status(&self, state: &mut CodexAppState) -> Result<(), CodexAppError> {
        if let Some(child) = state.child.as_mut() {
            if let Some(status) = child.try_wait()? {
                eprintln!("codex app-server exited with status {status}; restarting");
                state.child = None;
                state.session = None;
                state.thread_id = None;
            }
        }

        Ok(())
    }

    async fn ensure_server_started(&self, state: &mut CodexAppState) -> Result<(), CodexAppError> {
        if state.child.is_some() {
            return Ok(());
        }

        let child = Command::new("codex")
            .arg("app-server")
            .arg("--listen")
            .arg(&self.config.ws_url)
            .kill_on_drop(true)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        state.child = Some(child);
        Ok(())
    }

    async fn reset_session(&self) {
        let mut state = self.state.lock().await;
        state.session = None;
        state.thread_id = None;
    }
}

#[derive(Debug)]
struct CodexAppConfig {
    ws_url: String,
    model: String,
    effort: String,
    request_timeout: Duration,
    cwd: String,
}

impl CodexAppConfig {
    fn from_env() -> Self {
        let ws_url =
            std::env::var("PAIRPILOT_CODEX_APP_WS").unwrap_or_else(|_| DEFAULT_WS_URL.into());
        let model = std::env::var("PAIRPILOT_CODEX_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        let effort =
            std::env::var("PAIRPILOT_CODEX_EFFORT").unwrap_or_else(|_| DEFAULT_EFFORT.into());
        let timeout_ms = std::env::var("PAIRPILOT_CODEX_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_TIMEOUT_MS);
        let cwd = std::env::var("PAIRPILOT_CODEX_CWD").unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
                .display()
                .to_string()
        });

        Self {
            ws_url,
            model,
            effort,
            request_timeout: Duration::from_millis(timeout_ms),
            cwd,
        }
    }
}

#[derive(Default)]
struct CodexAppState {
    child: Option<Child>,
    session: Option<CodexAppSession>,
    thread_id: Option<String>,
}

struct CodexAppSession {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_request_id: u64,
    cwd: String,
}

impl CodexAppSession {
    async fn initialize(&mut self) -> Result<(), CodexAppError> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "pairpilot-daemon",
                    "title": "Pairpilot Daemon",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": true,
                    "requestAttestation": false,
                    "optOutNotificationMethods": [
                        "account/rateLimits/updated",
                        "mcpServer/startupStatus/updated"
                    ]
                }
            }),
        )
        .await?;
        Ok(())
    }

    async fn start_thread(&mut self, config: &CodexAppConfig) -> Result<String, CodexAppError> {
        let result = self
            .request(
                "thread/start",
                json!({
                    "cwd": self.cwd,
                    "runtimeWorkspaceRoots": [self.cwd],
                    "approvalPolicy": "never",
                    "sandbox": "read-only",
                    "model": config.model,
                    "serviceTier": null,
                    "baseInstructions": "You classify X/Twitter posts for Pairpilot. Return only the requested JSON.",
                    "developerInstructions": "Return concise valid JSON. Do not use tools. Do not run commands. Keep each opinion under 90 characters.",
                    "ephemeral": true,
                    "experimentalRawEvents": false,
                    "persistExtendedHistory": false
                }),
            )
            .await?;

        result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                CodexAppError::Protocol("thread/start response missing thread.id".into())
            })
    }

    async fn start_turn(
        &mut self,
        config: &CodexAppConfig,
        thread_id: &str,
        items: Vec<PromptItem>,
    ) -> Result<Vec<AiOpinion>, CodexAppError> {
        let prompt = build_prompt(&items)?;
        let schema = output_schema();
        let result = self
            .request(
                "turn/start",
                json!({
                    "threadId": thread_id,
                    "input": [
                        {
                            "type": "text",
                            "text": prompt,
                            "text_elements": []
                        }
                    ],
                    "cwd": self.cwd,
                    "runtimeWorkspaceRoots": [self.cwd],
                    "approvalPolicy": "never",
                    "sandboxPolicy": {
                        "type": "readOnly",
                        "networkAccess": false
                    },
                    "model": config.model,
                    "effort": config.effort,
                    "summary": "none",
                    "outputSchema": schema
                }),
            )
            .await?;

        let turn_id = result
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| CodexAppError::Protocol("turn/start response missing turn.id".into()))?;

        let final_text = self.read_turn_output(thread_id, &turn_id).await?;
        parse_opinions(&final_text)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, CodexAppError> {
        let id = self.next_request_id;
        self.next_request_id += 1;
        let request = json!({
            "id": id,
            "method": method,
            "params": params
        });

        self.socket
            .send(Message::Text(request.to_string().into()))
            .await?;

        loop {
            let message = self.next_json_message().await?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }

            if let Some(error) = message.get("error") {
                return Err(CodexAppError::Protocol(error.to_string()));
            }

            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    async fn read_turn_output(
        &mut self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<String, CodexAppError> {
        let mut streamed_text = String::new();
        let mut completed_text = None;

        loop {
            let message = self.next_json_message().await?;
            let method = message.get("method").and_then(Value::as_str);
            let params = message.get("params").unwrap_or(&Value::Null);

            match method {
                Some("item/agentMessage/delta")
                    if params.get("threadId").and_then(Value::as_str) == Some(thread_id)
                        && params.get("turnId").and_then(Value::as_str) == Some(turn_id) =>
                {
                    if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                        streamed_text.push_str(delta);
                    }
                }
                Some("item/completed")
                    if params.get("threadId").and_then(Value::as_str) == Some(thread_id)
                        && params.get("turnId").and_then(Value::as_str) == Some(turn_id) =>
                {
                    if params.pointer("/item/type").and_then(Value::as_str) == Some("agentMessage")
                    {
                        completed_text = params
                            .pointer("/item/text")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned);
                    }
                }
                Some("error")
                    if params.get("threadId").and_then(Value::as_str) == Some(thread_id)
                        && params.get("turnId").and_then(Value::as_str) == Some(turn_id) =>
                {
                    return Err(CodexAppError::Protocol(params.to_string()));
                }
                Some("turn/completed")
                    if params.get("threadId").and_then(Value::as_str) == Some(thread_id)
                        && params.pointer("/turn/id").and_then(Value::as_str) == Some(turn_id) =>
                {
                    if params.pointer("/turn/status").and_then(Value::as_str) == Some("failed") {
                        return Err(CodexAppError::Protocol(params.to_string()));
                    }

                    return Ok(completed_text.unwrap_or(streamed_text));
                }
                _ => {}
            }
        }
    }

    async fn next_json_message(&mut self) -> Result<Value, CodexAppError> {
        loop {
            let message = self
                .socket
                .next()
                .await
                .ok_or(CodexAppError::ConnectionClosed)??;

            match message {
                Message::Text(text) => return Ok(serde_json::from_str(&text)?),
                Message::Binary(bytes) => return Ok(serde_json::from_slice(&bytes)?),
                Message::Ping(payload) => {
                    self.socket.send(Message::Pong(payload)).await?;
                }
                Message::Close(_frame) => return Err(CodexAppError::ConnectionClosed),
                Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptItem {
    client_id: String,
    author: Option<String>,
    text: String,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpinionResponse {
    opinions: Vec<OpinionItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpinionItem {
    client_id: String,
    opinion: String,
    confidence: f32,
}

fn prompt_items(items: &[ContentItem]) -> Vec<PromptItem> {
    items
        .iter()
        .filter(|item| !item.text.trim().is_empty())
        .map(|item| PromptItem {
            client_id: item.client_id.clone(),
            author: item.author.clone(),
            text: item.text.clone(),
            url: item.url.clone(),
        })
        .collect()
}

fn build_prompt(items: &[PromptItem]) -> Result<String, CodexAppError> {
    let items_json = serde_json::to_string(items)?;
    Ok(format!(
        "For each X post below, give a short moderation opinion. Always return one opinion for every clientId. Use plain language. Do not hide, dim, or replace content. Return only JSON matching the schema. Posts JSON: {items_json}"
    ))
}

fn output_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "opinions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "clientId": { "type": "string" },
                        "opinion": { "type": "string" },
                        "confidence": { "type": "number" }
                    },
                    "required": ["clientId", "opinion", "confidence"]
                }
            }
        },
        "required": ["opinions"]
    })
}

fn parse_opinions(text: &str) -> Result<Vec<AiOpinion>, CodexAppError> {
    let parsed: OpinionResponse = serde_json::from_str(text.trim())?;
    Ok(parsed
        .opinions
        .into_iter()
        .map(|opinion| AiOpinion {
            client_id: opinion.client_id,
            opinion: opinion.opinion.trim().to_string(),
            confidence: opinion.confidence.clamp(0.0, 1.0),
        })
        .collect())
}

/// Error returned by the Codex app-server adapter.
#[derive(Debug)]
pub enum CodexAppError {
    /// Codex did not finish before the configured timeout.
    Timeout,
    /// The app-server websocket closed before a response arrived.
    ConnectionClosed,
    /// The app-server returned malformed or unsuccessful protocol data.
    Protocol(String),
    /// Local process or websocket I/O failed.
    Io(std::io::Error),
    /// Websocket connection or message handling failed.
    WebSocket(tokio_tungstenite::tungstenite::Error),
    /// Codex returned JSON that did not match the expected schema.
    Json(serde_json::Error),
}

impl fmt::Display for CodexAppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => write!(formatter, "request timed out"),
            Self::ConnectionClosed => write!(formatter, "connection closed"),
            Self::Protocol(message) => write!(formatter, "protocol error: {message}"),
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::WebSocket(error) => write!(formatter, "websocket error: {error}"),
            Self::Json(error) => write!(formatter, "json error: {error}"),
        }
    }
}

impl std::error::Error for CodexAppError {}

impl From<std::io::Error> for CodexAppError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for CodexAppError {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::WebSocket(error)
    }
}

impl From<serde_json::Error> for CodexAppError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
