use super::AppState;
use crate::{
    core::{
        DomAnalysisBatch, DomCommand, DomElementSnapshot, FeedbackContext, FeedbackKind,
        PageSnapshot,
    },
    sites,
};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

pub(super) async fn analyze_dom(
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

pub(super) async fn dom_feedback(
    State(state): State<AppState>,
    Json(request): Json<DomFeedbackRequest>,
) -> Json<DomFeedbackResponse> {
    let DomFeedbackRequest {
        feedback,
        page,
        element,
        reason,
        feedback_context,
    } = request;
    let batch = DomAnalysisBatch {
        page,
        elements: vec![element],
    };
    if state.log_captured_content {
        log_dom_batch(&batch);
    }

    Json(DomFeedbackResponse {
        commands: sites::apply_feedback(
            &batch,
            feedback,
            reason.as_str(),
            feedback_context,
            &state.content_store,
        ),
    })
}

pub(super) fn log_dom_batch(batch: &DomAnalysisBatch) {
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
    /// Rule context emitted by the daemon when the feedback control was rendered.
    pub feedback_context: FeedbackContext,
}

/// Response for a DOM feedback request.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomFeedbackResponse {
    /// Commands for the extension's generic DOM executor.
    pub commands: Vec<DomCommand>,
}
