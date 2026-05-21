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

const REVIEW_ALL_ENV: &str = "PAIRPILOT_X_REVIEW_ALL";

/// Analyzes X/Twitter timeline content.
pub async fn analyze(items: &[ContentItem], ai_analyzer: &AiAnalyzer) -> Vec<ContentDecision> {
    let review_all = env_flag(REVIEW_ALL_ENV);
    let ai_items: Vec<_> = items
        .iter()
        .filter(|item| should_ask_codex(item, review_all))
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
                    } else if should_ask_codex(item, review_all) {
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

fn should_ask_codex(item: &ContentItem, review_all: bool) -> bool {
    let normalized = item.text.to_lowercase();
    !item.text.trim().is_empty()
        && (review_all
            || count_matches(&normalized, SPAM_TERMS) > 0
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

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str) -> ContentItem {
        ContentItem {
            client_id: "test".into(),
            content_id: None,
            url: None,
            author: None,
            text: text.into(),
            captured_at: None,
            kind: Some("post".into()),
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn ordinary_posts_skip_codex_by_default() {
        assert!(!should_ask_codex(&item("Just a normal post."), false));
    }

    #[test]
    fn review_all_sends_ordinary_posts_to_codex() {
        assert!(should_ask_codex(&item("Just a normal post."), true));
    }

    #[test]
    fn empty_posts_do_not_go_to_codex_in_review_all_mode() {
        assert!(!should_ask_codex(&item("   "), true));
    }
}
