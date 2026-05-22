use crate::{
    ai::AiAnalyzer,
    core::{DomAnalysisBatch, DomCommand},
    sites,
    storage::{ContentStore, StorageError},
};
use axum::{
    extract::State,
    http::{header, Method},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

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
