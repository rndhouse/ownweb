use super::extract::ExtractedItem;
use crate::core::{ContentDecision, DomCommand, FeedbackContext};
use std::collections::HashMap;

pub(super) fn commands_from_decisions(
    extracted_items: Vec<ExtractedItem>,
    decisions: Vec<ContentDecision>,
    feedback_context: &FeedbackContext,
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
            let mut commands = vec![DomCommand::feedback_control_with_context(
                extracted.target.clone(),
                item_feedback_context,
            )];

            if let Some(decision) = decision {
                commands.push(DomCommand::from_decision(decision, extracted.target));
            }

            commands
        })
        .collect()
}
