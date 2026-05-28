use crate::{
    ai::AiAnalyzer,
    core::{DomAnalysisBatch, DomCommand, DomElementSnapshot, FeedbackKind, PageSnapshot},
    sites,
    storage::{
        ContentAnnotation, ContentAnnotationInput, ContentAnnotationQuery, ContentQuery,
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
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, info, warn};

const LOG_CAPTURED_CONTENT_ENV: &str = "WEBLAYER_LOG_CAPTURED_CONTENT";
const DEFAULT_CONTENT_LIMIT: usize = 100;
const MAX_CONTENT_LIMIT: usize = 500;
const DEFAULT_DISLIKE_LIMIT: usize = 100;
const MAX_DISLIKE_LIMIT: usize = 500;
const DEFAULT_RULE_LIMIT: usize = 100;
const MAX_RULE_LIMIT: usize = 500;
const DEFAULT_ANNOTATION_LIMIT: usize = 100;
const MAX_ANNOTATION_LIMIT: usize = 500;

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
        .route("/v1/content", get(content))
        .route(
            "/v1/content/annotations",
            get(content_annotations).post(upsert_content_annotation),
        )
        .route("/v1/content/stats", get(content_stats))
        .route("/v1/feedback", get(feedback))
        .route("/v1/dislikes", get(feedback))
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
        service: "weblayer-daemon",
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

async fn feedback(
    State(state): State<AppState>,
    Query(query): Query<FeedbackQuery>,
) -> Result<Json<FeedbackResponse>, ApiError> {
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

    Ok(Json(FeedbackResponse {
        site: site.as_str(),
        active,
        total_matching: page.total_matching,
        limit: page.limit,
        offset: page.offset,
        items: page.items,
    }))
}

async fn content(
    State(state): State<AppState>,
    Query(query): Query<ContentListQuery>,
) -> Result<Json<ContentListResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let search = query
        .q
        .map(|search| search.trim().to_string())
        .filter(|search| !search.is_empty());
    let limit = query
        .limit
        .unwrap_or(DEFAULT_CONTENT_LIMIT)
        .min(MAX_CONTENT_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let page = match site {
        SiteScope::XCom => state.content_store.x_content(ContentQuery {
            search: search.clone(),
            limit,
            offset,
        })?,
    };

    Ok(Json(ContentListResponse {
        site: site.as_str(),
        query: search,
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

async fn content_annotations(
    State(state): State<AppState>,
    Query(query): Query<ContentAnnotationsQuery>,
) -> Result<Json<ContentAnnotationsResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let storage_key = clean_query_value(query.storage_key);
    let content_id = clean_query_value(query.content_id);
    let content_kind = clean_query_value(query.content_kind);
    let annotation_type = clean_query_value(query.annotation_type);
    let key = clean_query_value(query.key);
    let source = clean_query_value(query.source);
    let limit = query
        .limit
        .unwrap_or(DEFAULT_ANNOTATION_LIMIT)
        .min(MAX_ANNOTATION_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let page = match site {
        SiteScope::XCom => state
            .content_store
            .x_content_annotations(ContentAnnotationQuery {
                storage_key: storage_key.clone(),
                content_id: content_id.clone(),
                content_kind: content_kind.clone(),
                annotation_type: annotation_type.clone(),
                key: key.clone(),
                source: source.clone(),
                limit,
                offset,
            })?,
    };

    Ok(Json(ContentAnnotationsResponse {
        site: site.as_str(),
        storage_key,
        content_id,
        content_kind,
        annotation_type,
        key,
        source,
        total_matching: page.total_matching,
        limit: page.limit,
        offset: page.offset,
        items: page.items,
    }))
}

async fn upsert_content_annotation(
    State(state): State<AppState>,
    Query(query): Query<ContentAnnotationSiteQuery>,
    Json(request): Json<UpsertContentAnnotationRequest>,
) -> Result<Json<UpsertContentAnnotationResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let storage_key = required_text(request.storage_key, "storageKey")?;
    let content_kind =
        clean_query_value(Some(request.content_kind)).unwrap_or_else(|| "post".into());
    let annotation_type = required_text(request.annotation_type, "annotationType")?;
    let key = request.key.trim().to_string();
    let source = required_text(request.source, "source")?;
    let confidence = validate_confidence(request.confidence)?;

    let annotation = match site {
        SiteScope::XCom => {
            state
                .content_store
                .x_upsert_content_annotation(ContentAnnotationInput {
                    storage_key,
                    content_kind,
                    annotation_type,
                    key,
                    value: request.value,
                    confidence,
                    source,
                })?
        }
    };

    Ok(Json(UpsertContentAnnotationResponse {
        site: site.as_str(),
        annotation,
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
                    target: "weblayer_daemon::captured_dom",
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
struct ContentListQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
    /// Optional full-text search query.
    q: Option<String>,
    /// Maximum number of rows to return. Defaults to 100 and is capped at 500.
    limit: Option<usize>,
    /// Number of matching rows to skip.
    offset: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContentListResponse {
    site: &'static str,
    query: Option<String>,
    total_matching: usize,
    limit: usize,
    offset: usize,
    items: Vec<crate::storage::StoredContentItem>,
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
struct ContentAnnotationsQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
    /// Optional stable storage key filter.
    storage_key: Option<String>,
    /// Optional site-native content ID filter.
    content_id: Option<String>,
    /// Optional logical content kind filter.
    content_kind: Option<String>,
    /// Optional annotation category filter.
    annotation_type: Option<String>,
    /// Optional annotation key filter.
    key: Option<String>,
    /// Optional source filter.
    source: Option<String>,
    /// Maximum number of rows to return. Defaults to 100 and is capped at 500.
    limit: Option<usize>,
    /// Number of matching rows to skip.
    offset: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContentAnnotationsResponse {
    site: &'static str,
    storage_key: Option<String>,
    content_id: Option<String>,
    content_kind: Option<String>,
    annotation_type: Option<String>,
    key: Option<String>,
    source: Option<String>,
    total_matching: usize,
    limit: usize,
    offset: usize,
    items: Vec<ContentAnnotation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContentAnnotationSiteQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertContentAnnotationRequest {
    /// Stable storage key returned by content inspection endpoints.
    storage_key: String,
    /// Logical content kind. Defaults to `post`.
    #[serde(default = "default_content_kind")]
    content_kind: String,
    /// Annotation category, such as `tag`, `note`, or `topic`.
    annotation_type: String,
    /// Annotation key within its category.
    #[serde(default)]
    key: String,
    /// Arbitrary annotation payload.
    value: Value,
    /// Optional model confidence from 0.0 to 1.0.
    confidence: Option<f64>,
    /// Source that created or updated this annotation.
    source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpsertContentAnnotationResponse {
    site: &'static str,
    annotation: ContentAnnotation,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FeedbackQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
    /// Filter by current active feedback state. Defaults to active feedback.
    active: Option<bool>,
    /// Maximum number of rows to return. Defaults to 100 and is capped at 500.
    limit: Option<usize>,
    /// Number of matching rows to skip.
    offset: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedbackResponse {
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

fn default_content_kind() -> String {
    "post".into()
}

fn clean_query_value(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn required_text(value: String, field: &str) -> Result<String, ApiError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(ApiError::bad_request(format!("{field} must not be empty")));
    }

    Ok(value)
}

fn validate_confidence(confidence: Option<f64>) -> Result<Option<f64>, ApiError> {
    let Some(confidence) = confidence else {
        return Ok(None);
    };

    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err(ApiError::bad_request(
            "confidence must be between 0.0 and 1.0",
        ));
    }

    Ok(Some(confidence))
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
