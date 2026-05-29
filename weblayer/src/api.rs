mod content;
mod dom;
mod error;
mod events;
mod feedback;
mod input;
mod rules;
mod site;

use crate::{
    ai::AiAnalyzer,
    storage::{ContentStore, StorageError},
};
use axum::{
    http::{header, Method},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};

const LOG_CAPTURED_CONTENT_ENV: &str = "WEBLAYER_LOG_CAPTURED_CONTENT";

/// Builds the daemon HTTP router.
pub fn router() -> Result<Router, StorageError> {
    let state = AppState {
        ai_analyzer: AiAnalyzer::from_env(),
        content_store: ContentStore::from_env()?,
        log_captured_content: captured_content_logging_enabled(),
    };

    Ok(Router::new()
        .route("/health", get(health))
        .route("/v1/events", get(events::events_ws))
        .route("/v1/dom/analyze", post(dom::analyze_dom))
        .route("/v1/dom/feedback", post(dom::dom_feedback))
        .route("/v1/content", get(content::content))
        .route(
            "/v1/content/annotations",
            get(content::content_annotations).post(content::upsert_content_annotation),
        )
        .route("/v1/content/stats", get(content::content_stats))
        .route("/v1/feedback", get(feedback::feedback))
        .route("/v1/dislikes", get(feedback::feedback))
        .route(
            "/v1/rule-proposals",
            get(rules::rule_set_proposals).post(rules::create_rule_set_proposal),
        )
        .route(
            "/v1/rule-proposals/{proposal_id}",
            get(rules::rule_set_proposal),
        )
        .route("/v1/rule-suggestions", get(rules::rule_suggestions))
        .route("/v1/rules", get(rules::rules).post(rules::create_rule))
        .route(
            "/v1/rules/{rule_id}",
            get(rules::rule_detail).post(rules::update_rule),
        )
        .route(
            "/v1/rules/{rule_id}/status",
            post(rules::update_rule_status),
        )
        .route("/v1/rules/{rule_id}/validate", get(rules::validate_rule))
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
