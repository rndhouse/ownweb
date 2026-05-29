use super::{AiAction, AiContentRule, AiOpinion};
use crate::{
    core::ContentItem,
    storage::{ContentRule, RuleSetProposalChange, XDislikedPost},
};
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
use tracing::warn;

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:39177";
const DEFAULT_MODEL: &str = "gpt-5.3-codex-spark";
const DEFAULT_EFFORT: &str = "low";
const DEFAULT_TIMEOUT_MS: u64 = 8000;
const WS_ENV: &str = "WEBLAYER_CODEX_APP_WS";
const MODEL_ENV: &str = "WEBLAYER_CODEX_MODEL";
const EFFORT_ENV: &str = "WEBLAYER_CODEX_EFFORT";
const TIMEOUT_ENV: &str = "WEBLAYER_CODEX_TIMEOUT_MS";
const CWD_ENV: &str = "WEBLAYER_CODEX_CWD";

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
    pub async fn x_opinions(
        &self,
        items: &[ContentItem],
        rules: &[AiContentRule],
    ) -> Result<Vec<AiOpinion>, CodexAppError> {
        let prompt_items = prompt_items(items);
        if prompt_items.is_empty() {
            return Ok(Vec::new());
        }
        let prompt_rules = prompt_rules(rules);

        match timeout(
            self.config.request_timeout,
            self.run_x_opinion_turn(prompt_items, prompt_rules),
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

    /// Gets proposed X rule-set changes from feedback and current active rules.
    pub async fn x_rule_set_proposal(
        &self,
        feedback: &[XDislikedPost],
        active_rules: &[ContentRule],
    ) -> Result<Vec<RuleSetProposalChange>, CodexAppError> {
        let prompt_feedback = prompt_feedback(feedback);
        let prompt_rules = prompt_rule_set_rules(active_rules);

        match timeout(
            self.config.request_timeout,
            self.run_x_rule_set_proposal_turn(prompt_feedback, prompt_rules),
        )
        .await
        {
            Ok(Ok(changes)) => Ok(changes),
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
        prompt_rules: Vec<PromptRule>,
    ) -> Result<Vec<AiOpinion>, CodexAppError> {
        let prompt = build_prompt(&prompt_items, &prompt_rules)?;
        let schema = output_schema();
        let final_text = self.run_turn(prompt, schema).await?;

        parse_opinions(&final_text)
    }

    async fn run_x_rule_set_proposal_turn(
        &self,
        feedback: Vec<PromptFeedback>,
        active_rules: Vec<PromptRuleSetRule>,
    ) -> Result<Vec<RuleSetProposalChange>, CodexAppError> {
        let prompt = build_rule_set_proposal_prompt(&feedback, &active_rules)?;
        let schema = rule_set_proposal_output_schema();
        let final_text = self.run_turn(prompt, schema).await?;

        parse_rule_set_proposal(&final_text)
    }

    async fn run_turn(&self, prompt: String, schema: Value) -> Result<String, CodexAppError> {
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
            .start_turn(&self.config, &thread_id, prompt, schema)
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
                warn!(%status, "codex app-server exited; restarting");
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
        let ws_url = env_var(WS_ENV).unwrap_or_else(|| DEFAULT_WS_URL.into());
        let model = env_var(MODEL_ENV).unwrap_or_else(|| DEFAULT_MODEL.into());
        let effort = env_var(EFFORT_ENV).unwrap_or_else(|| DEFAULT_EFFORT.into());
        let timeout_ms = env_var(TIMEOUT_ENV)
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_TIMEOUT_MS);
        let cwd = env_var(CWD_ENV).unwrap_or_else(|| {
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

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
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
                    "name": "weblayer-daemon",
                    "title": "WebLayer Daemon",
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
                    "baseInstructions": "You support WebLayer by evaluating X/Twitter content and curating user-defined filtering rules. Return only the requested JSON.",
                    "developerInstructions": "Return concise valid JSON matching the provided schema. Do not use tools. Do not run commands. Follow the task instructions exactly.",
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
        prompt: String,
        schema: Value,
    ) -> Result<String, CodexAppError> {
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
        Ok(final_text)
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptRule {
    id: String,
    priority: i64,
    title: String,
    instruction: String,
    positive_examples: Vec<String>,
    negative_examples: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptRuleSetRule {
    id: String,
    status: String,
    priority: i64,
    title: String,
    instruction: String,
    positive_examples: Vec<String>,
    negative_examples: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptFeedback {
    storage_key: String,
    post_id: Option<String>,
    url: Option<String>,
    author: Option<String>,
    text: String,
    reason: String,
    rules_at_feedback: Vec<PromptFeedbackRule>,
    decision_action_at_feedback: Option<String>,
    matched_rule_ids_at_feedback: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptFeedbackRule {
    id: String,
    priority: i64,
    title: String,
    instruction: String,
    positive_examples: Vec<String>,
    negative_examples: Vec<String>,
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
    action: OpinionAction,
    opinion: String,
    confidence: f32,
    matched_rule_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum OpinionAction {
    Keep,
    Hide,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuleSetProposalResponse {
    changes: Vec<RuleSetProposalChange>,
}

fn prompt_items(items: &[ContentItem]) -> Vec<PromptItem> {
    items
        .iter()
        .filter(|item| {
            !item.text.trim().is_empty()
                || item
                    .url
                    .as_deref()
                    .is_some_and(|url| !url.trim().is_empty())
        })
        .map(|item| PromptItem {
            client_id: item.client_id.clone(),
            author: item.author.clone(),
            text: item.text.clone(),
            url: item.url.clone(),
        })
        .collect()
}

fn prompt_rules(rules: &[AiContentRule]) -> Vec<PromptRule> {
    rules
        .iter()
        .map(|rule| PromptRule {
            id: rule.id.clone(),
            priority: rule.priority,
            title: rule.title.clone(),
            instruction: rule.instruction.clone(),
            positive_examples: rule.positive_examples.clone(),
            negative_examples: rule.negative_examples.clone(),
        })
        .collect()
}

fn prompt_rule_set_rules(rules: &[ContentRule]) -> Vec<PromptRuleSetRule> {
    rules
        .iter()
        .map(|rule| PromptRuleSetRule {
            id: rule.id.clone(),
            status: rule.status.clone(),
            priority: rule.priority,
            title: rule.title.clone(),
            instruction: rule.instruction.clone(),
            positive_examples: rule.examples.positive.clone(),
            negative_examples: rule.examples.negative.clone(),
        })
        .collect()
}

fn prompt_feedback(feedback: &[XDislikedPost]) -> Vec<PromptFeedback> {
    feedback
        .iter()
        .map(|item| {
            let decision = item.rule_context.decision.as_ref();
            PromptFeedback {
                storage_key: item.storage_key.clone(),
                post_id: item.post_id.clone(),
                url: item.url.clone(),
                author: item.author.clone(),
                text: item.text.clone(),
                reason: item.reason.clone(),
                rules_at_feedback: item
                    .rule_context
                    .active_rules
                    .iter()
                    .map(|rule| PromptFeedbackRule {
                        id: rule.id.clone(),
                        priority: rule.priority,
                        title: rule.title.clone(),
                        instruction: rule.instruction.clone(),
                        positive_examples: rule.positive_examples.clone(),
                        negative_examples: rule.negative_examples.clone(),
                    })
                    .collect(),
                decision_action_at_feedback: decision.map(|decision| decision.action.clone()),
                matched_rule_ids_at_feedback: decision
                    .map(|decision| decision.matched_rule_ids.clone())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

fn build_prompt(items: &[PromptItem], rules: &[PromptRule]) -> Result<String, CodexAppError> {
    let items_json = serde_json::to_string(items)?;
    let rules_json = serde_json::to_string(rules)?;
    Ok(format!(
        "Evaluate each X post against the active user rules. Active rules JSON: {rules_json}\nPosts JSON: {items_json}\nReturn one opinion for every clientId. The only valid actions are \"keep\" and \"hide\". Use \"hide\" only when the post clearly matches one or more active rules; include those rule IDs in matchedRuleIds. Use \"keep\" when no active rule clearly matches, and return an empty matchedRuleIds array. Do not dim, label, or replace content. Keep opinion under 120 characters and explain the decision in plain language. Return only JSON matching the schema."
    ))
}

fn build_rule_set_proposal_prompt(
    feedback: &[PromptFeedback],
    active_rules: &[PromptRuleSetRule],
) -> Result<String, CodexAppError> {
    let feedback_json = serde_json::to_string(feedback)?;
    let active_rules_json = serde_json::to_string(active_rules)?;
    Ok(format!(
        "Create a reviewable rule-set proposal for WebLayer X filtering. Current active rules JSON: {active_rules_json}\nActive user feedback JSON: {feedback_json}\nReconcile the feedback with the current active rules. Use the feedback-time rulesAtFeedback and matchedRuleIds to distinguish uncovered feedback from feedback that already had a rule in play. Avoid duplicate rules and avoid broad overlapping rules. Prefer updateRule when feedback is clearly evidence for an existing active rule. Use createRule only for a coherent uncovered theme, and set status to \"draft\". Use disableRule only when an active rule is redundant with another active rule or clearly obsolete. Use noChange when feedback is too sparse or already covered. For updateRule, include the existing ruleId and only fields that should change; include positive examples when feedback should become rule evidence. For disableRule, include the existing ruleId and status \"disabled\". Put feedback storage keys in evidenceStorageKeys. Keep rationales under 160 characters. Return only JSON matching the schema."
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
                        "action": { "type": "string", "enum": ["keep", "hide"] },
                        "opinion": { "type": "string" },
                        "confidence": { "type": "number" },
                        "matchedRuleIds": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    },
                    "required": ["clientId", "action", "opinion", "confidence", "matchedRuleIds"]
                }
            }
        },
        "required": ["opinions"]
    })
}

fn rule_set_proposal_output_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "changes": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["createRule", "updateRule", "disableRule", "noChange"]
                        },
                        "ruleId": { "type": ["string", "null"] },
                        "status": {
                            "type": ["string", "null"],
                            "enum": ["draft", "active", "disabled", "archived", null]
                        },
                        "priority": { "type": ["integer", "null"] },
                        "title": { "type": ["string", "null"] },
                        "instruction": { "type": ["string", "null"] },
                        "rationale": { "type": "string" },
                        "evidenceStorageKeys": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "examples": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "positive": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                },
                                "negative": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                }
                            },
                            "required": ["positive", "negative"]
                        }
                    },
                    "required": [
                        "action",
                        "ruleId",
                        "status",
                        "priority",
                        "title",
                        "instruction",
                        "rationale",
                        "evidenceStorageKeys",
                        "examples"
                    ]
                }
            }
        },
        "required": ["changes"]
    })
}

fn parse_opinions(text: &str) -> Result<Vec<AiOpinion>, CodexAppError> {
    let parsed: OpinionResponse = serde_json::from_str(text.trim())?;
    Ok(parsed
        .opinions
        .into_iter()
        .map(|opinion| AiOpinion {
            client_id: opinion.client_id,
            action: match opinion.action {
                OpinionAction::Keep => AiAction::Keep,
                OpinionAction::Hide => AiAction::Hide,
            },
            opinion: opinion.opinion.trim().to_string(),
            confidence: opinion.confidence.clamp(0.0, 1.0),
            matched_rule_ids: opinion.matched_rule_ids,
        })
        .collect())
}

fn parse_rule_set_proposal(text: &str) -> Result<Vec<RuleSetProposalChange>, CodexAppError> {
    let parsed: RuleSetProposalResponse = serde_json::from_str(text.trim())?;
    Ok(parsed.changes)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_includes_active_rules() {
        let items = vec![PromptItem {
            client_id: "client-1".into(),
            author: Some("@alice".into()),
            text: "look at this absurd video".into(),
            url: Some("https://x.com/alice/status/123".into()),
        }];
        let rules = vec![PromptRule {
            id: "x-engagement-bait-reaction".into(),
            priority: 50,
            title: "Engagement bait reaction posts".into(),
            instruction: "Downrank engagement bait reaction posts.".into(),
            positive_examples: vec!["lol this clip is ridiculous".into()],
            negative_examples: vec!["here is a specific claim".into()],
        }];

        let prompt = build_prompt(&items, &rules).expect("prompt should build");

        assert!(prompt.contains("Active rules JSON"));
        assert!(prompt.contains("x-engagement-bait-reaction"));
        assert!(prompt.contains("look at this absurd video"));
        assert!(prompt.contains("The only valid actions are \"keep\" and \"hide\""));
    }

    #[test]
    fn parse_opinions_reads_hide_actions_and_rule_ids() {
        let opinions = parse_opinions(
            r#"{
                "opinions": [
                    {
                        "clientId": "client-1",
                        "action": "hide",
                        "opinion": "Matches engagement-bait reaction rule.",
                        "confidence": 1.2,
                        "matchedRuleIds": ["x-engagement-bait-reaction"]
                    }
                ]
            }"#,
        )
        .expect("opinion json should parse");

        assert_eq!(opinions.len(), 1);
        assert_eq!(opinions[0].client_id, "client-1");
        assert_eq!(opinions[0].action, AiAction::Hide);
        assert_eq!(opinions[0].confidence, 1.0);
        assert_eq!(
            opinions[0].matched_rule_ids,
            vec!["x-engagement-bait-reaction"]
        );
    }

    #[test]
    fn rule_set_proposal_prompt_includes_feedback_context() {
        let feedback = vec![PromptFeedback {
            storage_key: "x:id:123".into(),
            post_id: Some("123".into()),
            url: Some("https://x.com/alice/status/123".into()),
            author: Some("@alice".into()),
            text: "reply yes if you agree".into(),
            reason: "engagement bait".into(),
            rules_at_feedback: vec![PromptFeedbackRule {
                id: "x-engagement-bait-reaction".into(),
                priority: 50,
                title: "Engagement bait reaction posts".into(),
                instruction: "Downrank engagement bait reaction posts.".into(),
                positive_examples: Vec::new(),
                negative_examples: Vec::new(),
            }],
            decision_action_at_feedback: Some("keep".into()),
            matched_rule_ids_at_feedback: Vec::new(),
        }];
        let active_rules = vec![PromptRuleSetRule {
            id: "x-engagement-bait-reaction".into(),
            status: "active".into(),
            priority: 50,
            title: "Engagement bait reaction posts".into(),
            instruction: "Downrank engagement bait reaction posts.".into(),
            positive_examples: Vec::new(),
            negative_examples: Vec::new(),
        }];

        let prompt =
            build_rule_set_proposal_prompt(&feedback, &active_rules).expect("prompt should build");

        assert!(prompt.contains("Current active rules JSON"));
        assert!(prompt.contains("rulesAtFeedback"));
        assert!(prompt.contains("matchedRuleIds"));
        assert!(prompt.contains("reply yes if you agree"));
        assert!(prompt.contains("Prefer updateRule"));
    }

    #[test]
    fn parse_rule_set_proposal_reads_update_changes() {
        let changes = parse_rule_set_proposal(
            r#"{
                "changes": [
                    {
                        "action": "updateRule",
                        "ruleId": "x-engagement-bait-reaction",
                        "status": "active",
                        "priority": 50,
                        "title": "Engagement bait reaction posts",
                        "instruction": "Hide engagement bait reaction posts.",
                        "rationale": "Feedback is best handled by tightening the existing rule.",
                        "evidenceStorageKeys": ["x:id:123"],
                        "examples": {
                            "positive": ["reply yes if you agree"],
                            "negative": []
                        }
                    }
                ]
            }"#,
        )
        .expect("proposal json should parse");

        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].rule_id.as_deref(),
            Some("x-engagement-bait-reaction")
        );
        assert_eq!(changes[0].examples.positive, vec!["reply yes if you agree"]);
    }
}
