mod commands;
mod decide;
mod extract;
#[cfg(test)]
mod tests;

use crate::{
    ai::{AiAnalyzer, AiContentRule},
    core::{
        AnalysisBatch, DomAnalysisBatch, DomCommand, FeedbackContext, FeedbackKind,
        FeedbackRuleContext,
    },
    storage::{ContentStore, StorageError},
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
    let feedback_context = feedback_context_from_active_rules(&active_rules);
    let decisions = decide::decide_items(&content_batch.items, ai_analyzer, &active_rules).await;
    let decisions = decide::apply_stored_feedback(content_store, &content_batch.items, decisions);
    decide::record_decision_events(
        content_store,
        &content_batch.items,
        &decisions,
        "domAnalysis",
    );
    commands::commands_from_decisions(extracted_items, decisions, &feedback_context, content_store)
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
    let feedback_context = feedback_context_from_active_rules(&active_rules);
    let decisions = decide::cached_decide_items(&content_batch.items, ai_analyzer, &active_rules)?;
    let decisions = decide::apply_stored_feedback(content_store, &content_batch.items, decisions);
    record_content_batch(content_store, &content_batch);
    decide::record_decision_events(
        content_store,
        &content_batch.items,
        &decisions,
        "cachedDomAnalysis",
    );

    Some(commands::commands_from_decisions(
        extracted_items,
        decisions,
        &feedback_context,
        content_store,
    ))
}

/// Returns immediate commands that install controls for identified X posts.
pub fn pending_dom_commands(
    batch: &DomAnalysisBatch,
    _ai_analyzer: &AiAnalyzer,
    content_store: &ContentStore,
) -> Vec<DomCommand> {
    let active_rules = decide::active_x_rules(content_store);
    let feedback_context = feedback_context_from_active_rules(&active_rules);

    extract::extract_items(batch)
        .into_iter()
        .filter_map(|extracted| {
            if let Some(decision) = decide::stored_dislike_decision(content_store, &extracted.item)
            {
                Some(DomCommand::from_decision(
                    decision,
                    extracted.target.clone(),
                ))
            } else {
                feedback_control_with_stored_context(
                    content_store,
                    extracted.target,
                    &feedback_context,
                )
            }
        })
        .collect()
}

/// Applies user feedback to X/Twitter DOM snapshots.
pub fn apply_feedback(
    batch: &DomAnalysisBatch,
    feedback: FeedbackKind,
    reason: &str,
    feedback_context_id: &str,
    content_store: &ContentStore,
) -> Result<Vec<DomCommand>, StorageError> {
    let extracted_items = extract::extract_items(batch);
    if extracted_items.is_empty() {
        return Ok(Vec::new());
    }

    let content_batch = content_batch_from_extracted(&extracted_items);
    record_content_batch(content_store, &content_batch);
    let feedback_context = content_store
        .x_feedback_context(feedback_context_id)?
        .ok_or_else(|| {
            StorageError::InvalidInput(format!("feedback context not found: {feedback_context_id}"))
        })?;
    decide::record_feedback(
        content_store,
        &content_batch.items,
        feedback,
        reason,
        &feedback_context,
    )?;

    Ok(Vec::new())
}

fn feedback_control_with_stored_context(
    content_store: &ContentStore,
    target: crate::core::DomCommandTarget,
    feedback_context: &FeedbackContext,
) -> Option<DomCommand> {
    match content_store.store_x_feedback_context(feedback_context) {
        Ok(context_id) => Some(DomCommand::feedback_control_with_context_id(
            target, context_id,
        )),
        Err(error) => {
            warn!(%error, "failed to store feedback context for X pending command");
            None
        }
    }
}

fn feedback_context_from_active_rules(active_rules: &[AiContentRule]) -> FeedbackContext {
    FeedbackContext {
        active_rules: active_rules
            .iter()
            .map(|rule| FeedbackRuleContext {
                id: rule.id.clone(),
                priority: rule.priority,
                title: rule.title.clone(),
                instruction: rule.instruction.clone(),
                updated_at_unix_ms: rule.updated_at_unix_ms,
                positive_examples: rule.positive_examples.clone(),
                negative_examples: rule.negative_examples.clone(),
            })
            .collect(),
        decision: None,
    }
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
