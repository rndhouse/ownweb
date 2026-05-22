use crate::{
    ai::AiAnalyzer,
    core::{DomAnalysisBatch, DomCommand},
    sites,
    storage::{ContentStore, StorageError},
};
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::State,
    http::{header, Method},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, info, warn};

const LOG_CAPTURED_CONTENT_ENV: &str = "OWNWEB_LOG_CAPTURED_CONTENT";

/// Builds the daemon HTTP router.
pub fn router() -> Result<Router, StorageError> {
    let state = AppState {
        ai_analyzer: AiAnalyzer::from_env(),
        content_store: ContentStore::from_env()?,
        log_captured_content: captured_content_logging_enabled(),
    };

    Ok(Router::new()
        .route("/health", get(health))
        .route("/v1/events", get(events_ws))
        .route("/v1/dom/analyze", post(analyze_dom))
        .with_state(state)
        .layer(cors_layer()))
}

/// Returns whether captured DOM snapshots should be emitted as structured logs.
pub fn captured_content_logging_enabled() -> bool {
    env_flag_default(LOG_CAPTURED_CONTENT_ENV, false)
}

fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE])
        .max_age(Duration::from_secs(60 * 60))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "ownweb-daemon",
    })
}

async fn events_ws(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_event_socket(socket, state))
}

async fn analyze_dom(
    State(state): State<AppState>,
    Json(batch): Json<DomAnalysisBatch>,
) -> Json<DomAnalyzeResponse> {
    if state.log_captured_content {
        log_dom_batch(&batch);
    }

    Json(DomAnalyzeResponse {
        commands: sites::analyze_dom(&batch, &state.ai_analyzer, &state.content_store).await,
    })
}

async fn handle_event_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let (event_sender, mut event_receiver) = mpsc::unbounded_channel::<ServerEvent>();

    let writer = tokio::spawn(async move {
        while let Some(event) = event_receiver.recv().await {
            let Ok(json) = serde_json::to_string(&event) else {
                warn!("failed to serialize WebSocket event");
                continue;
            };

            if let Err(error) = sender.send(Message::Text(json.into())).await {
                debug!(%error, "failed to send WebSocket event");
                break;
            }
        }
    });

    while let Some(message) = receiver.next().await {
        match message {
            Ok(Message::Text(text)) => {
                handle_client_event(text.as_str(), state.clone(), event_sender.clone()).await;
            }
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Binary(_)) => {}
            Err(error) => {
                debug!(%error, "WebSocket receive failed");
                break;
            }
        }
    }

    drop(event_sender);
    if let Err(error) = writer.await {
        debug!(%error, "WebSocket writer task failed");
    }
}

async fn handle_client_event(
    text: &str,
    state: AppState,
    event_sender: mpsc::UnboundedSender<ServerEvent>,
) {
    let event = match serde_json::from_str::<ClientEvent>(text) {
        Ok(event) => event,
        Err(error) => {
            warn!(%error, "failed to parse WebSocket event");
            return;
        }
    };

    match event {
        ClientEvent::AnalyzeDom {
            request_id,
            page,
            elements,
        } => {
            let batch = DomAnalysisBatch { page, elements };
            if state.log_captured_content {
                log_dom_batch(&batch);
            }

            let pending_commands = sites::pending_dom_commands(&batch);
            if !pending_commands.is_empty() {
                let _ = event_sender.send(ServerEvent::commands(
                    request_id.clone(),
                    AnalysisPhase::Pending,
                    pending_commands,
                ));
            }

            let final_sender = event_sender.clone();
            tokio::spawn(async move {
                let commands =
                    sites::analyze_dom(&batch, &state.ai_analyzer, &state.content_store).await;
                let _ = final_sender.send(ServerEvent::commands(
                    request_id,
                    AnalysisPhase::Final,
                    commands,
                ));
            });
        }
    }
}

fn log_dom_batch(batch: &DomAnalysisBatch) {
    let received_at_unix_ms = now_unix_ms();

    for element in &batch.elements {
        match serde_json::to_string(element) {
            Ok(element_json) => {
                info!(
                    target: "ownweb_daemon::captured_dom",
                    page_url = batch.page.url.as_str(),
                    client_id = element.client_id.as_str(),
                    selector = element.selector.as_deref(),
                    snapshot_hash = element.snapshot_hash.as_deref(),
                    received_at_unix_ms,
                    element = %element_json,
                    "captured DOM region"
                );
            }
            Err(error) => {
                warn!(%error, "failed to serialize DOM snapshot for logging");
            }
        }
    }
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ClientEvent {
    AnalyzeDom {
        #[serde(rename = "requestId")]
        request_id: String,
        page: crate::core::PageSnapshot,
        #[serde(default)]
        elements: Vec<crate::core::DomElementSnapshot>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerEvent {
    r#type: &'static str,
    request_id: String,
    phase: AnalysisPhase,
    commands: Vec<DomCommand>,
}

impl ServerEvent {
    fn commands(request_id: String, phase: AnalysisPhase, commands: Vec<DomCommand>) -> Self {
        Self {
            r#type: "commands",
            request_id,
            phase,
            commands,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum AnalysisPhase {
    Pending,
    Final,
}

/// Response for the DOM snapshot analysis endpoint.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomAnalyzeResponse {
    /// Commands for the extension's generic DOM executor.
    pub commands: Vec<DomCommand>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
}

#[derive(Clone)]
struct AppState {
    ai_analyzer: AiAnalyzer,
    content_store: ContentStore,
    log_captured_content: bool,
}

fn env_flag_default(name: &str, default: bool) -> bool {
    std::env::var(name)
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(default)
}
