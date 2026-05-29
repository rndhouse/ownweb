use crate::{
    ai::{AiAction, AiAnalyzer, AiContentRule, AiOpinion},
    core::{ContentDecision, ContentItem, FeedbackContext, FeedbackKind},
    storage::{ContentStore, RuleQuery},
};
use std::collections::HashMap;
use tracing::warn;

pub(super) fn record_feedback(
    content_store: &ContentStore,
    items: &[ContentItem],
    feedback: FeedbackKind,
    reason: &str,
    feedback_context: &FeedbackContext,
) {
    for item in items {
        if let Err(error) =
            content_store.record_x_feedback_with_context(item, feedback, reason, feedback_context)
        {
            warn!(
                %error,
                client_id = item.client_id.as_str(),
                content_id = item.content_id.as_deref(),
                "failed to store X feedback"
            );
        }
    }
}

pub(super) async fn decide_items(
    items: &[ContentItem],
    ai_analyzer: &AiAnalyzer,
    active_rules: &[AiContentRule],
) -> Vec<ContentDecision> {
    let ai_items: Vec<_> = items
        .iter()
        .filter(|item| should_ask_codex(item))
        .cloned()
        .collect();

    if !ai_items.is_empty() {
        if let Some(opinions) = ai_analyzer.x_opinions(&ai_items, active_rules).await {
            let mut opinions_by_id: HashMap<_, _> = opinions
                .into_iter()
                .map(|opinion| (opinion.client_id.clone(), opinion))
                .collect();

            return items
                .iter()
                .map(|item| {
                    if let Some(opinion) = opinions_by_id.remove(&item.client_id) {
                        reviewed_item_decision(opinion)
                    } else {
                        ContentDecision::keep(item.client_id.clone())
                    }
                })
                .collect();
        }
    }

    items
        .iter()
        .map(|item| ContentDecision::keep(item.client_id.clone()))
        .collect()
}

pub(super) fn apply_stored_feedback(
    content_store: &ContentStore,
    items: &[ContentItem],
    decisions: Vec<ContentDecision>,
) -> Vec<ContentDecision> {
    let items_by_client_id: HashMap<&str, &ContentItem> = items
        .iter()
        .map(|item| (item.client_id.as_str(), item))
        .collect();

    decisions
        .into_iter()
        .map(|decision| {
            let Some(item) = items_by_client_id.get(decision.client_id.as_str()) else {
                return decision;
            };

            stored_dislike_decision(content_store, item).unwrap_or(decision)
        })
        .collect()
}

pub(super) fn stored_dislike_decision(
    content_store: &ContentStore,
    item: &ContentItem,
) -> Option<ContentDecision> {
    match content_store.x_feedback_state(item) {
        Ok(Some(state)) if state.active => Some(ContentDecision::hide(
            item.client_id.clone(),
            "WebLayer: hidden",
            "Previously disliked",
            1.0,
        )),
        Ok(_) => None,
        Err(error) => {
            warn!(
                %error,
                client_id = item.client_id.as_str(),
                content_id = item.content_id.as_deref(),
                "failed to read X feedback state"
            );
            None
        }
    }
}

pub(super) fn cached_decide_items(
    items: &[ContentItem],
    ai_analyzer: &AiAnalyzer,
    active_rules: &[AiContentRule],
) -> Option<Vec<ContentDecision>> {
    let ai_items: Vec<_> = items
        .iter()
        .filter(|item| should_ask_codex(item))
        .cloned()
        .collect();
    let mut opinions_by_id: HashMap<_, _> = if ai_items.is_empty() {
        HashMap::new()
    } else {
        ai_analyzer
            .cached_x_opinions(&ai_items, active_rules)?
            .into_iter()
            .map(|opinion| (opinion.client_id.clone(), opinion))
            .collect()
    };
    let mut decisions = Vec::with_capacity(items.len());

    for item in items {
        if let Some(opinion) = opinions_by_id.remove(&item.client_id) {
            decisions.push(reviewed_item_decision(opinion));
        } else if should_ask_codex(item) {
            return None;
        } else {
            decisions.push(ContentDecision::keep(item.client_id.clone()));
        }
    }

    Some(decisions)
}

pub(super) fn active_x_rules(content_store: &ContentStore) -> Vec<AiContentRule> {
    match content_store.x_rules(RuleQuery {
        status: Some("active".into()),
        ..RuleQuery::default()
    }) {
        Ok(page) => page.items.into_iter().map(AiContentRule::from).collect(),
        Err(error) => {
            warn!(%error, "failed to load active X rules");
            Vec::new()
        }
    }
}

pub(super) fn reviewed_item_decision(opinion: AiOpinion) -> ContentDecision {
    match opinion.action {
        AiAction::Keep => ContentDecision::keep(opinion.client_id),
        AiAction::Hide => ContentDecision::hide(
            opinion.client_id,
            "WebLayer: hidden by rule",
            opinion.opinion,
            opinion.confidence,
        )
        .with_matched_rule_ids(opinion.matched_rule_ids),
    }
}

pub(super) fn should_ask_codex(item: &ContentItem) -> bool {
    has_prompt_content(item)
}

fn has_prompt_content(item: &ContentItem) -> bool {
    !item.text.trim().is_empty()
        || item
            .url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty())
}
