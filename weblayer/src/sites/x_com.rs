mod commands;
mod decide;
mod extract;
#[cfg(test)]
mod tests;

use crate::{
    ai::AiAnalyzer,
    core::{AnalysisBatch, DomAnalysisBatch, DomCommand, FeedbackKind},
    storage::ContentStore,
};
use tracing::warn;

/// Interprets X/Twitter DOM snapshots and returns browser DOM commands.
pub async fn analyze_dom(
    batch: &DomAnalysisBatch,
    ai_analyzer: &AiAnalyzer,
    content_store: &ContentStore,
) -> Vec<DomCommand> {
    let extracted_items = extract::extract_items(batch);
    if extracted_items.is_empty() {
        return Vec::new();
    }

    let content_batch = content_batch_from_extracted(&extracted_items);
    record_content_batch(content_store, &content_batch);

    let active_rules = decide::active_x_rules(content_store);
    let decisions = decide::decide_items(&content_batch.items, ai_analyzer, &active_rules).await;
    let decisions = decide::apply_stored_feedback(content_store, &content_batch.items, decisions);
    commands::commands_from_decisions(extracted_items, decisions)
}

/// Returns final commands when every required X summary is already cached.
pub fn cached_dom_commands(
    batch: &DomAnalysisBatch,
    ai_analyzer: &AiAnalyzer,
    content_store: &ContentStore,
) -> Option<Vec<DomCommand>> {
    let extracted_items = extract::extract_items(batch);
    if extracted_items.is_empty() {
        return Some(Vec::new());
    }

    let content_batch = content_batch_from_extracted(&extracted_items);
    let active_rules = decide::active_x_rules(content_store);
    let decisions = decide::cached_decide_items(&content_batch.items, ai_analyzer, &active_rules)?;
    let decisions = decide::apply_stored_feedback(content_store, &content_batch.items, decisions);
    record_content_batch(content_store, &content_batch);

    Some(commands::commands_from_decisions(
        extracted_items,
        decisions,
    ))
}

/// Returns immediate commands that install controls for identified X posts.
pub fn pending_dom_commands(
    batch: &DomAnalysisBatch,
    _ai_analyzer: &AiAnalyzer,
    content_store: &ContentStore,
) -> Vec<DomCommand> {
    extract::extract_items(batch)
        .into_iter()
        .map(|extracted| {
            decide::stored_dislike_decision(content_store, &extracted.item)
                .map(|decision| DomCommand::from_decision(decision, extracted.target.clone()))
                .unwrap_or_else(|| DomCommand::feedback_control(extracted.target))
        })
        .collect()
}

/// Applies user feedback to X/Twitter DOM snapshots.
pub fn apply_feedback(
    batch: &DomAnalysisBatch,
    feedback: FeedbackKind,
    reason: &str,
    content_store: &ContentStore,
) -> Vec<DomCommand> {
    let extracted_items = extract::extract_items(batch);
    if extracted_items.is_empty() {
        return Vec::new();
    }

    let content_batch = content_batch_from_extracted(&extracted_items);
    record_content_batch(content_store, &content_batch);
    decide::record_feedback(content_store, &content_batch.items, feedback, reason);

    Vec::new()
}

fn content_batch_from_extracted(extracted_items: &[extract::ExtractedItem]) -> AnalysisBatch {
    AnalysisBatch::new(
        "x.com",
        extracted_items
            .iter()
            .map(|extracted| extracted.item.clone())
            .collect(),
    )
}

fn record_content_batch(content_store: &ContentStore, content_batch: &AnalysisBatch) {
    if let Err(error) = content_store.record_x_batch(content_batch) {
        warn!(%error, "failed to store X content");
    }
}
