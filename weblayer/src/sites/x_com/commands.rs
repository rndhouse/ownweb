use super::extract::ExtractedItem;
use crate::core::{ContentDecision, DomCommand, FeedbackContext};
use crate::storage::ContentStore;
use std::collections::HashMap;
use tracing::warn;

pub(super) fn commands_from_decisions(
    extracted_items: Vec<ExtractedItem>,
    decisions: Vec<ContentDecision>,
    feedback_context: &FeedbackContext,
    content_store: &ContentStore,
) -> Vec<DomCommand> {
    let mut decisions_by_id: HashMap<_, _> = decisions
        .into_iter()
        .map(|decision| (decision.client_id.clone(), decision))
        .collect();

    extracted_items
        .into_iter()
        .flat_map(|extracted| {
            let decision = decisions_by_id.remove(&extracted.item.client_id);
            let item_feedback_context = decision
                .as_ref()
                .map(|decision| feedback_context.with_decision(decision))
                .unwrap_or_else(|| feedback_context.clone());
            let mut commands = feedback_control_command(
                content_store,
                extracted.target.clone(),
                &item_feedback_context,
            )
            .into_iter()
            .collect::<Vec<_>>();

            if let Some(decision) = decision {
                commands.push(DomCommand::from_decision(decision, extracted.target));
            }

            commands
        })
        .collect()
}

fn feedback_control_command(
    content_store: &ContentStore,
    target: crate::core::DomCommandTarget,
    feedback_context: &FeedbackContext,
) -> Option<DomCommand> {
    match content_store.store_x_feedback_context(feedback_context) {
        Ok(context_id) => Some(DomCommand::feedback_control_with_context_id(
            target, context_id,
        )),
        Err(error) => {
            warn!(%error, "failed to store feedback context for X command");
            None
        }
    }
}
