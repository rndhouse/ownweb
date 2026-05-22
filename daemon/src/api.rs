use crate::{
    ai::AiAnalyzer,
    core::{DomAnalysisBatch, DomCommand, DomElementSnapshot, FeedbackKind, PageSnapshot},
    sites,
    storage::{
        ContentRule, ContentStats, ContentStore, RuleQuery, StorageError, XDislikeQuery,
        XDislikedPost,
    },
};
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Query, State},
    http::{header, Method, StatusCode},
    response::{IntoResponse, Response},
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
const DEFAULT_DISLIKE_LIMIT: usize = 100;
const MAX_DISLIKE_LIMIT: usize = 500;
const DEFAULT_RULE_LIMIT: usize = 100;
const MAX_RULE_LIMIT: usize = 500;

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
        .route("/v1/dom/feedback", post(dom_feedback))
        .route("/v1/content/stats", get(content_stats))
        .route("/v1/dislikes", get(dislikes))
        .route("/v1/rules", get(rules))
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

async fn dom_feedback(
    State(state): State<AppState>,
    Json(request): Json<DomFeedbackRequest>,
) -> Json<DomFeedbackResponse> {
    let DomFeedbackRequest {
        feedback,
        page,
        element,
        reason,
    } = request;
    let batch = DomAnalysisBatch {
        page,
        elements: vec![element],
    };
    if state.log_captured_content {
        log_dom_batch(&batch);
    }

    Json(DomFeedbackResponse {
        commands: sites::apply_feedback(&batch, feedback, reason.as_str(), &state.content_store),
    })
}

async fn dislikes(
    State(state): State<AppState>,
    Query(query): Query<DislikesQuery>,
) -> Result<Json<DislikesResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let active = query.active.or(Some(true));
    let limit = query
        .limit
        .unwrap_or(DEFAULT_DISLIKE_LIMIT)
        .min(MAX_DISLIKE_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let page = match site {
        SiteScope::XCom => state.content_store.x_dislikes(XDislikeQuery {
            active,
            limit,
            offset,
        })?,
    };

    Ok(Json(DislikesResponse {
        site: site.as_str(),
        active,
        total_matching: page.total_matching,
        limit: page.limit,
        offset: page.offset,
        items: page.items,
    }))
}

async fn content_stats(
    State(state): State<AppState>,
    Query(query): Query<ContentStatsQuery>,
) -> Result<Json<ContentStatsResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let stats = match site {
        SiteScope::XCom => state.content_store.x_content_stats()?,
    };

    Ok(Json(ContentStatsResponse {
        site: site.as_str(),
        stats,
    }))
}

async fn rules(
    State(state): State<AppState>,
    Query(query): Query<RulesQuery>,
) -> Result<Json<RulesResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let status = query
        .status
        .map(|status| status.trim().to_string())
        .filter(|status| !status.is_empty());
    let limit = query
        .limit
        .unwrap_or(DEFAULT_RULE_LIMIT)
        .min(MAX_RULE_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let page = match site {
        SiteScope::XCom => state.content_store.x_rules(RuleQuery {
            status: status.clone(),
            limit,
            offset,
        })?,
    };

    Ok(Json(RulesResponse {
        site: site.as_str(),
        status,
        total_matching: page.total_matching,
        limit: page.limit,
        offset: page.offset,
        items: page.items,
    }))
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

            if let Some(commands) =
                sites::cached_dom_commands(&batch, &state.ai_analyzer, &state.content_store)
            {
                let _ = event_sender.send(ServerEvent::commands(
                    request_id,
                    AnalysisPhase::Final,
                    commands,
                ));
                return;
            }

            let _ = event_sender.send(ServerEvent::commands(
                request_id.clone(),
                AnalysisPhase::Pending,
                sites::pending_dom_commands(&batch, &state.ai_analyzer, &state.content_store),
            ));

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

/// Request for applying user feedback to one captured DOM region.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomFeedbackRequest {
    /// Feedback signal chosen by the user.
    pub feedback: FeedbackKind,
    /// Optional user-supplied reason for the feedback.
    #[serde(default)]
    pub reason: String,
    /// Snapshot metadata for the live page.
    pub page: PageSnapshot,
    /// DOM region that received feedback.
    pub element: DomElementSnapshot,
}

/// Response for a DOM feedback request.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomFeedbackResponse {
    /// Commands for the extension's generic DOM executor.
    pub commands: Vec<DomCommand>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContentStatsQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContentStatsResponse {
    site: &'static str,
    stats: ContentStats,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DislikesQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
    /// Filter by current active dislike state. Defaults to active dislikes.
    active: Option<bool>,
    /// Maximum number of rows to return. Defaults to 100 and is capped at 500.
    limit: Option<usize>,
    /// Number of matching rows to skip.
    offset: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DislikesResponse {
    site: &'static str,
    active: Option<bool>,
    total_matching: usize,
    limit: usize,
    offset: usize,
    items: Vec<XDislikedPost>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RulesQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
    /// Optional rule status filter.
    status: Option<String>,
    /// Maximum number of rows to return. Defaults to 100 and is capped at 500.
    limit: Option<usize>,
    /// Number of matching rows to skip.
    offset: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RulesResponse {
    site: &'static str,
    status: Option<String>,
    total_matching: usize,
    limit: usize,
    offset: usize,
    items: Vec<ContentRule>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
}

impl From<StorageError> for ApiError {
    fn from(error: StorageError) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SiteScope {
    XCom,
}

impl SiteScope {
    fn from_param(site: Option<&str>) -> Result<Self, ApiError> {
        match site.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) if is_x_site(value) => Ok(Self::XCom),
            Some(value) => Err(ApiError::bad_request(format!(
                "unsupported site query parameter: {value}"
            ))),
            None => Err(ApiError::bad_request(
                "missing required site query parameter",
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::XCom => "x.com",
        }
    }
}

fn is_x_site(site: &str) -> bool {
    site.eq_ignore_ascii_case("x.com") || site.eq_ignore_ascii_case("twitter.com")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_scope_accepts_supported_site_query_values() {
        assert_eq!(
            SiteScope::from_param(Some("x.com")).unwrap(),
            SiteScope::XCom
        );
        assert_eq!(
            SiteScope::from_param(Some(" twitter.com ")).unwrap(),
            SiteScope::XCom
        );
    }

    #[test]
    fn site_scope_rejects_missing_site_query() {
        let error = SiteScope::from_param(None).unwrap_err();

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.message, "missing required site query parameter");
    }

    #[test]
    fn site_scope_rejects_unsupported_site_query() {
        let error = SiteScope::from_param(Some("example.com")).unwrap_err();

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            error.message,
            "unsupported site query parameter: example.com"
        );
    }
}
