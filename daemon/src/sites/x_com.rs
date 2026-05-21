use crate::core::{ContentDecision, ContentItem};

/// Analyzes X/Twitter timeline content.
pub fn analyze(items: &[ContentItem]) -> Vec<ContentDecision> {
    items.iter().map(classify_item).collect()
}

fn classify_item(item: &ContentItem) -> ContentDecision {
    let normalized = item.text.to_lowercase();
    let spam_hits = count_matches(
        &normalized,
        &[
            "airdrop",
            "crypto giveaway",
            "guaranteed returns",
            "100x",
            "dm me",
            "link in bio",
            "free money",
        ],
    );
    let ai_hits = count_matches(
        &normalized,
        &[
            "as an ai",
            "delve",
            "unlock the power",
            "game-changer",
            "in today's fast-paced",
            "revolutionize",
            "seamlessly",
        ],
    );

    if item.text.trim().is_empty() {
        return ContentDecision::keep(item.client_id.clone());
    }

    if spam_hits >= 2 {
        return ContentDecision::hide(
            item.client_id.clone(),
            "Pairpilot: spam",
            "Matched promotional spam heuristics",
            0.9,
        );
    }

    if ai_hits >= 2 {
        return ContentDecision::dim(
            item.client_id.clone(),
            "Pairpilot: likely generated",
            "Matched generated-writing heuristics",
            0.72,
        );
    }

    ContentDecision::keep(item.client_id.clone())
}

fn count_matches(text: &str, terms: &[&str]) -> usize {
    terms.iter().filter(|term| text.contains(**term)).count()
}
