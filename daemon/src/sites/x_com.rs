use crate::{
    ai::AiAnalyzer,
    core::{
        AnalysisBatch, ContentDecision, ContentItem, DomAnalysisBatch, DomCommand,
        DomCommandTarget, DomElementSnapshot,
    },
    storage::ContentStore,
};
use serde_json::json;
use std::collections::HashMap;
use tracing::warn;

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

/// Interprets X/Twitter DOM snapshots and returns browser DOM commands.
pub async fn analyze_dom(
    batch: &DomAnalysisBatch,
    ai_analyzer: &AiAnalyzer,
    content_store: &ContentStore,
) -> Vec<DomCommand> {
    let extracted_items = extract_items(batch);
    if extracted_items.is_empty() {
        return Vec::new();
    }

    let content_batch = AnalysisBatch::new(
        "x.com",
        extracted_items
            .iter()
            .map(|extracted| extracted.item.clone())
            .collect(),
    );
    if let Err(error) = content_store.record_batch(&content_batch) {
        warn!(%error, "failed to store X content");
    }

    let decisions = decide_items(&content_batch.items, ai_analyzer).await;
    commands_from_decisions(extracted_items, decisions)
}

async fn decide_items(items: &[ContentItem], ai_analyzer: &AiAnalyzer) -> Vec<ContentDecision> {
    let review_all = env_flag_default(REVIEW_ALL_ENV, true);
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

fn extract_items(batch: &DomAnalysisBatch) -> Vec<ExtractedItem> {
    batch
        .elements
        .iter()
        .filter_map(|element| extract_item(batch, element))
        .collect()
}

fn extract_item(batch: &DomAnalysisBatch, element: &DomElementSnapshot) -> Option<ExtractedItem> {
    let status_href = find_status_href(element).or_else(|| status_href_from_page(&batch.page.url));
    let post_id = status_href
        .as_deref()
        .and_then(x_status_id)
        .map(ToOwned::to_owned);

    if post_id.is_none() {
        return None;
    }

    let author = status_href.as_deref().and_then(author_handle);
    let item = ContentItem {
        client_id: element.client_id.clone(),
        content_id: post_id,
        url: status_href,
        author,
        text: element.text.clone(),
        captured_at: element
            .captured_at
            .clone()
            .or_else(|| batch.page.captured_at.clone()),
        kind: Some("post".into()),
        metadata: json!({
            "pageUrl": batch.page.url,
            "pageTitle": batch.page.title,
            "selector": element.selector,
            "tagName": element.tag_name,
            "role": element.role,
            "snapshotHash": element.snapshot_hash,
        }),
    };
    let target = DomCommandTarget {
        client_id: element.client_id.clone(),
        selector: element.selector.clone(),
        must_match_snapshot_hash: element.snapshot_hash.clone(),
    };

    Some(ExtractedItem { item, target })
}

fn commands_from_decisions(
    extracted_items: Vec<ExtractedItem>,
    decisions: Vec<ContentDecision>,
) -> Vec<DomCommand> {
    let mut decisions_by_id: HashMap<_, _> = decisions
        .into_iter()
        .map(|decision| (decision.client_id.clone(), decision))
        .collect();

    extracted_items
        .into_iter()
        .filter_map(|extracted| {
            decisions_by_id
                .remove(&extracted.item.client_id)
                .map(|decision| DomCommand::from_decision(decision, extracted.target))
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

fn find_status_href(element: &DomElementSnapshot) -> Option<String> {
    element
        .links
        .iter()
        .find_map(|link| x_status_id(&link.href).map(|_| link.href.clone()))
}

fn status_href_from_page(url: &str) -> Option<String> {
    x_status_id(url).map(|_| url.to_string())
}

fn x_status_id(url: &str) -> Option<&str> {
    let marker = "/status/";
    let start = url.find(marker)? + marker.len();
    let rest = &url[start..];
    let end = rest
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(rest.len());

    (end > 0).then_some(&rest[..end])
}

fn author_handle(url: &str) -> Option<String> {
    let without_scheme = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
    let path = without_scheme.split_once('/')?.1;
    let handle = path.split_once("/status/")?.0.trim_matches('/');

    (!handle.is_empty()).then(|| format!("@{handle}"))
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

fn env_flag_default(name: &str, default: bool) -> bool {
    std::env::var(name)
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(default)
}

struct ExtractedItem {
    item: ContentItem,
    target: DomCommandTarget,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{DomLink, PageSnapshot};

    fn batch(elements: Vec<DomElementSnapshot>) -> DomAnalysisBatch {
        DomAnalysisBatch {
            page: PageSnapshot {
                url: "https://x.com/home".into(),
                title: Some("X".into()),
                captured_at: Some("2026-05-22T09:00:00.000Z".into()),
            },
            elements,
        }
    }

    fn element(client_id: &str, text: &str, href: Option<&str>) -> DomElementSnapshot {
        DomElementSnapshot {
            client_id: client_id.into(),
            selector: Some("article:nth-of-type(1)".into()),
            tag_name: Some("article".into()),
            role: None,
            text: text.into(),
            html: None,
            attributes: Vec::new(),
            links: href
                .map(|href| {
                    vec![DomLink {
                        href: href.into(),
                        text: Some("status".into()),
                        aria_label: None,
                    }]
                })
                .unwrap_or_default(),
            snapshot_hash: Some("hash1".into()),
            captured_at: None,
        }
    }

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
    fn extracts_x_post_from_dom_snapshot_link() {
        let batch = batch(vec![element(
            "client-1",
            "A noisy article containing a post",
            Some("https://x.com/alice/status/12345?s=20"),
        )]);

        let extracted = extract_items(&batch);

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].item.content_id.as_deref(), Some("12345"));
        assert_eq!(extracted[0].item.author.as_deref(), Some("@alice"));
        assert_eq!(
            extracted[0].target.must_match_snapshot_hash.as_deref(),
            Some("hash1")
        );
    }

    #[test]
    fn ignores_x_dom_regions_without_status_identity() {
        let batch = batch(vec![element("client-1", "navigation", None)]);

        assert!(extract_items(&batch).is_empty());
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
