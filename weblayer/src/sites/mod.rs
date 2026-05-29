use crate::{
    ai::AiAnalyzer,
    core::{DomAnalysisBatch, DomCommand, FeedbackContext, FeedbackKind},
    storage::ContentStore,
};

pub mod x_com;

/// Returns immediate browser commands for content that should be gated.
pub fn pending_dom_commands(
    batch: &DomAnalysisBatch,
    ai_analyzer: &AiAnalyzer,
    content_store: &ContentStore,
) -> Vec<DomCommand> {
    match page_host(&batch.page.url).as_deref() {
        Some("x.com") | Some("twitter.com") => {
            x_com::pending_dom_commands(batch, ai_analyzer, content_store)
        }
        _ => Vec::new(),
    }
}

/// Returns final DOM commands when they can be produced without async analysis.
pub fn cached_dom_commands(
    batch: &DomAnalysisBatch,
    ai_analyzer: &AiAnalyzer,
    content_store: &ContentStore,
) -> Option<Vec<DomCommand>> {
    match page_host(&batch.page.url).as_deref() {
        Some("x.com") | Some("twitter.com") => {
            x_com::cached_dom_commands(batch, ai_analyzer, content_store)
        }
        _ => Some(Vec::new()),
    }
}

/// Applies user feedback through the matching site handler.
pub fn apply_feedback(
    batch: &DomAnalysisBatch,
    feedback: FeedbackKind,
    reason: &str,
    feedback_context: FeedbackContext,
    content_store: &ContentStore,
) -> Vec<DomCommand> {
    match page_host(&batch.page.url).as_deref() {
        Some("x.com") | Some("twitter.com") => {
            x_com::apply_feedback(batch, feedback, reason, feedback_context, content_store)
        }
        _ => Vec::new(),
    }
}

/// Dispatches a DOM snapshot to the matching site handler.
pub async fn analyze_dom(
    batch: &DomAnalysisBatch,
    ai_analyzer: &AiAnalyzer,
    content_store: &ContentStore,
) -> Vec<DomCommand> {
    match page_host(&batch.page.url).as_deref() {
        Some("x.com") | Some("twitter.com") => {
            x_com::analyze_dom(batch, ai_analyzer, content_store).await
        }
        _ => Vec::new(),
    }
}

fn page_host(url: &str) -> Option<String> {
    let without_scheme = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
    let host = without_scheme.split('/').next()?.split(':').next()?.trim();

    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}
