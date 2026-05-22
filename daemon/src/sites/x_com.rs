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

const REVIEW_ALL_ENV: &str = "OWNWEB_X_REVIEW_ALL";
const LEGACY_REVIEW_ALL_ENV: &str = "PAIRPILOT_X_REVIEW_ALL";

/// Analyzes X/Twitter timeline content.
pub async fn analyze(items: &[ContentItem], ai_analyzer: &AiAnalyzer) -> Vec<ContentDecision> {
    let review_all = env_flag_default(REVIEW_ALL_ENV, LEGACY_REVIEW_ALL_ENV, true);
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
                            format!("Summary: {}", opinion.opinion),
                            "Codex app-server summary",
                            opinion.confidence,
                        )
                    } else if review_all && has_prompt_content(item) {
                        summary_unavailable(item)
                    } else if should_ask_codex(item, review_all) {
                        classify_item(item)
                    } else {
                        ContentDecision::keep(item.client_id.clone())
                    }
                })
                .collect();
        }
    }

    items
        .iter()
        .map(|item| {
            if review_all && has_prompt_content(item) {
                summary_unavailable(item)
            } else {
                classify_item(item)
            }
        })
        .collect()
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
            "OwnWeb: spam",
            "Matched promotional spam heuristics",
            0.9,
        );
    }

    if ai_hits >= 2 {
        return ContentDecision::dim(
            item.client_id.clone(),
            "OwnWeb: likely generated",
            "Matched generated-writing heuristics",
            0.72,
        );
    }

    ContentDecision::keep(item.client_id.clone())
}

fn should_ask_codex(item: &ContentItem, review_all: bool) -> bool {
    let normalized = item.text.to_lowercase();
    has_prompt_content(item)
        && (review_all
            || count_matches(&normalized, SPAM_TERMS) > 0
            || count_matches(&normalized, AI_TERMS) > 0
            || count_matches(&normalized, REVIEW_TERMS) > 0
            || has_url_signal(&normalized))
}

fn has_prompt_content(item: &ContentItem) -> bool {
    !item.text.trim().is_empty()
        || item
            .url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty())
}

fn summary_unavailable(item: &ContentItem) -> ContentDecision {
    ContentDecision::label(
        item.client_id.clone(),
        "Summary: Codex summary unavailable",
        "Codex app-server did not return a summary",
        0.0,
    )
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

fn env_flag_default(name: &str, legacy_name: &str, default: bool) -> bool {
    std::env::var(name)
        .or_else(|_| std::env::var(legacy_name))
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str, url: Option<&str>) -> ContentItem {
        ContentItem {
            client_id: "test".into(),
            content_id: None,
            url: url.map(ToOwned::to_owned),
            author: None,
            text: text.into(),
            captured_at: None,
            kind: Some("post".into()),
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn ordinary_posts_can_skip_codex_when_review_all_is_disabled() {
        assert!(!should_ask_codex(&item("Just a normal post.", None), false));
    }

    #[test]
    fn review_all_sends_ordinary_posts_to_codex_for_summaries() {
        assert!(should_ask_codex(&item("Just a normal post.", None), true));
    }

    #[test]
    fn review_all_can_send_url_only_posts_to_codex() {
        assert!(should_ask_codex(
            &item("   ", Some("https://x.com/user/status/1")),
            true
        ));
    }

    #[test]
    fn empty_posts_without_url_do_not_go_to_codex_in_review_all_mode() {
        assert!(!should_ask_codex(&item("   ", None), true));
    }
}
