use crate::{
    ai::AiAnalyzer,
    core::{AnalysisBatch, ContentDecision, ContentItem},
    sites,
    storage::{ContentStore, StorageError},
};
use axum::{
    extract::State,
    http::{header, Method},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

const LOG_CAPTURED_CONTENT_ENV: &str = "PAIRPILOT_LOG_CAPTURED_CONTENT";

/// Builds the daemon HTTP router.
pub fn router() -> Result<Router, StorageError> {
    let state = AppState {
        ai_analyzer: AiAnalyzer::from_env(),
        content_store: ContentStore::from_env()?,
        log_captured_content: captured_content_logging_enabled(),
    };

    Ok(Router::new()
        .route("/health", get(health))
        .route("/v1/content/analyze", post(analyze_content))
        .route("/v1/x-posts/analyze", post(analyze_x_posts))
        .with_state(state)
        .layer(cors_layer()))
}

/// Returns whether captured content should be emitted as structured log events.
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
        service: "pairpilot-daemon",
    })
}

async fn analyze_content(
    State(state): State<AppState>,
    Json(request): Json<ContentAnalyzeRequest>,
) -> Json<ContentAnalyzeResponse> {
    let batch = request.into_batch();
    analyze_batch(&state, &batch, ContentAnalyzeResponse::new).await
}

async fn analyze_x_posts(
    State(state): State<AppState>,
    Json(request): Json<XPostsAnalyzeRequest>,
) -> Json<XPostsAnalyzeResponse> {
    let batch = request.into_batch();
    analyze_batch(&state, &batch, XPostsAnalyzeResponse::new).await
}

async fn analyze_batch<T>(
    state: &AppState,
    batch: &AnalysisBatch,
    build_response: impl FnOnce(Vec<ContentDecision>) -> T,
) -> Json<T> {
    if state.log_captured_content {
        log_captured_batch(batch);
    }
    if let Err(error) = state.content_store.record_batch(batch) {
        warn!(%error, "failed to store captured content");
    }

    Json(build_response(
        sites::analyze(batch, &state.ai_analyzer).await,
    ))
}

fn log_captured_batch(batch: &AnalysisBatch) {
    if batch.items.is_empty() {
        return;
    }

    let received_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    for item in &batch.items {
        let captured_item = CapturedItem {
            received_at_unix_ms,
            source: batch.source.as_str(),
            item,
        };
        match serde_json::to_string(&captured_item) {
            Ok(captured_json) => {
                info!(
                    target: "pairpilot_daemon::captured_content",
                    source = batch.source.as_str(),
                    client_id = item.client_id.as_str(),
                    content_id = item.content_id.as_deref(),
                    received_at_unix_ms,
                    captured = %captured_json,
                    "captured content"
                );
            }
            Err(error) => {
                warn!(%error, "failed to serialize captured content for logging");
            }
        }
    }
}

/// Request for the generic content analysis endpoint.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentAnalyzeRequest {
    /// Site or integration source that produced the content.
    #[serde(default)]
    pub source: Option<String>,
    /// Normalized content items to analyze.
    pub items: Vec<ContentItem>,
}

impl ContentAnalyzeRequest {
    fn into_batch(self) -> AnalysisBatch {
        AnalysisBatch::new(self.source.unwrap_or_else(|| "unknown".into()), self.items)
    }
}

/// Response for the generic content analysis endpoint.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentAnalyzeResponse {
    /// Decisions keyed by each item's `clientId`.
    pub items: Vec<ContentDecision>,
}

impl ContentAnalyzeResponse {
    fn new(items: Vec<ContentDecision>) -> Self {
        Self { items }
    }
}

/// Backwards-compatible request for the X post analysis endpoint.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XPostsAnalyzeRequest {
    /// Site source. Defaults to `x.com`.
    #[serde(default)]
    pub source: Option<String>,
    /// X posts to analyze.
    pub posts: Vec<XPostItem>,
}

impl XPostsAnalyzeRequest {
    fn into_batch(self) -> AnalysisBatch {
        let items = self.posts.into_iter().map(ContentItem::from).collect();
        AnalysisBatch::new(self.source.unwrap_or_else(|| "x.com".into()), items)
    }
}

/// Backwards-compatible response for the X post analysis endpoint.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XPostsAnalyzeResponse {
    /// Decisions keyed by each post's `clientId`.
    pub posts: Vec<ContentDecision>,
}

impl XPostsAnalyzeResponse {
    fn new(posts: Vec<ContentDecision>) -> Self {
        Self { posts }
    }
}

/// X-specific post payload accepted by the compatibility endpoint.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XPostItem {
    /// Client-generated ID used to update the matching DOM element.
    pub client_id: String,
    /// X status ID when available.
    #[serde(default)]
    pub post_id: Option<String>,
    /// Canonical post URL when available.
    #[serde(default)]
    pub url: Option<String>,
    /// X author handle when available.
    #[serde(default)]
    pub author_handle: Option<String>,
    /// User-visible post text.
    #[serde(default)]
    pub text: String,
    /// Client-side capture timestamp.
    #[serde(default)]
    pub captured_at: Option<String>,
}

impl From<XPostItem> for ContentItem {
    fn from(post: XPostItem) -> Self {
        Self {
            client_id: post.client_id,
            content_id: post.post_id,
            url: post.url,
            author: post.author_handle,
            text: post.text,
            captured_at: post.captured_at,
            kind: Some("post".into()),
            metadata: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CapturedItem<'a> {
    received_at_unix_ms: u128,
    source: &'a str,
    item: &'a ContentItem,
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
