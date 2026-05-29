use super::extract::ExtractedItem;
use crate::core::{ContentDecision, DomCommand};
use std::collections::HashMap;

pub(super) fn commands_from_decisions(
    extracted_items: Vec<ExtractedItem>,
    decisions: Vec<ContentDecision>,
) -> Vec<DomCommand> {
    let mut decisions_by_id: HashMap<_, _> = decisions
        .into_iter()
        .map(|decision| (decision.client_id.clone(), decision))
        .collect();

    extracted_items
        .into_iter()
        .flat_map(|extracted| {
            let mut commands = vec![DomCommand::feedback_control(extracted.target.clone())];

            if let Some(decision) = decisions_by_id.remove(&extracted.item.client_id) {
                commands.push(DomCommand::from_decision(decision, extracted.target));
            }

            commands
        })
        .collect()
}
