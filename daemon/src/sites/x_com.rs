use crate::{
    ai::AiAnalyzer,
    core::{ContentDecision, ContentItem},
};
use std::collections::HashMap;

const SPAM_TERMS: &[&str] = &[
    "airdrop",
    "crypto giveaway",
    "guaranteed returns",
    "100x",
    "dm me",
    "link in bio",
    "free money",
];

const AI_TERMS: &[&str] = &[
    "as an ai",
    "delve",
    "unlock the power",
    "game-changer",
    "in today's fast-paced",
    "revolutionize",
    "seamlessly",
];

const REVIEW_TERMS: &[&str] = &[
    "giveaway",
    "promo",
    "discount",
    "limited time",
    "claim now",
    "click here",
    "subscribe",
    "join my",
    "join our",
    "discord.gg",
    "t.me/",
    "telegram",
    "whatsapp",
    "onlyfans",
    "patreon",
];

/// Analyzes X/Twitter timeline content.
pub async fn analyze(items: &[ContentItem], ai_analyzer: &AiAnalyzer) -> Vec<ContentDecision> {
    let ai_items: Vec<_> = items
        .iter()
        .filter(|item| should_ask_codex(item))
        .cloned()
        .collect();

    if !ai_items.is_empty() {
        if let Some(opinions) = ai_analyzer.x_opinions(&ai_items).await {
            let mut opinions_by_id: HashMap<_, _> = opinions
                .into_iter()
                .map(|opinion| (opinion.client_id.clone(), opinion))
                .collect();

            return items
                .iter()
                .map(|item| {
                    if let Some(opinion) = opinions_by_id.remove(&item.client_id) {
                        ContentDecision::label(
                            opinion.client_id,
                            format!("Codex: {}", opinion.opinion),
                            "Codex app-server opinion",
                            opinion.confidence,
                        )
                    } else if should_ask_codex(item) {
                        classify_item(item)
                    } else {
                        ContentDecision::keep(item.client_id.clone())
                    }
                })
                .collect();
        }
    }

    items.iter().map(classify_item).collect()
}

fn classify_item(item: &ContentItem) -> ContentDecision {
    let normalized = item.text.to_lowercase();
    let spam_hits = count_matches(&normalized, SPAM_TERMS);
    let ai_hits = count_matches(&normalized, AI_TERMS);

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

fn should_ask_codex(item: &ContentItem) -> bool {
    let normalized = item.text.to_lowercase();
    !item.text.trim().is_empty()
        && (count_matches(&normalized, SPAM_TERMS) > 0
            || count_matches(&normalized, AI_TERMS) > 0
            || count_matches(&normalized, REVIEW_TERMS) > 0
            || has_url_signal(&normalized))
}

fn has_url_signal(text: &str) -> bool {
    text.contains("http://")
        || text.contains("https://")
        || text.contains("www.")
        || text.contains(".com")
        || text.contains(".io")
        || text.contains(".xyz")
}

fn count_matches(text: &str, terms: &[&str]) -> usize {
    terms.iter().filter(|term| text.contains(**term)).count()
}
