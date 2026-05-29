use super::{
    apply_feedback,
    decide::{cached_decide_items, reviewed_item_decision, should_ask_codex},
    extract::extract_items,
    pending_dom_commands,
};
use crate::{
    ai::{AiAction, AiAnalyzer, AiOpinion},
    core::{
        ContentItem, DecisionAction, DomAnalysisBatch, DomAttribute, DomElementSnapshot, DomLink,
        FeedbackContext, FeedbackKind, PageSnapshot,
    },
    storage::{ContentStore, RuleCreateInput, RuleExamples},
};
use std::path::PathBuf;

fn batch(elements: Vec<DomElementSnapshot>) -> DomAnalysisBatch {
    batch_with_url("https://x.com/home", elements)
}

fn batch_with_url(url: &str, elements: Vec<DomElementSnapshot>) -> DomAnalysisBatch {
    DomAnalysisBatch {
        page: PageSnapshot {
            url: url.into(),
            title: Some("X".into()),
            captured_at: Some("2026-05-22T09:00:00.000Z".into()),
        },
        elements,
    }
}

fn element(client_id: &str, text: &str, href: Option<&str>) -> DomElementSnapshot {
    element_with_root(client_id, "article", None, text, href)
}

fn element_with_root(
    client_id: &str,
    tag_name: &str,
    role: Option<&str>,
    text: &str,
    href: Option<&str>,
) -> DomElementSnapshot {
    DomElementSnapshot {
        client_id: client_id.into(),
        selector: Some(format!("{tag_name}:nth-of-type(1)")),
        tag_name: Some(tag_name.into()),
        role: role.map(ToOwned::to_owned),
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

fn data_testid_tweet_element(
    client_id: &str,
    text: &str,
    href: Option<&str>,
) -> DomElementSnapshot {
    let mut element = element_with_root(client_id, "div", None, text, href);
    element.attributes = vec![DomAttribute {
        name: "data-testid".into(),
        value: "tweet".into(),
    }];
    element
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

fn content_item(client_id: &str, content_id: &str, text: &str) -> ContentItem {
    ContentItem {
        client_id: client_id.into(),
        content_id: Some(content_id.into()),
        url: Some(format!("https://x.com/user/status/{content_id}")),
        author: Some("@user".into()),
        text: text.into(),
        captured_at: None,
        kind: Some("post".into()),
        metadata: serde_json::Value::Null,
    }
}

fn temp_data_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "weblayer-x-site-test-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

fn content_store(name: &str) -> (ContentStore, PathBuf) {
    let data_dir = temp_data_dir(name);
    let store = ContentStore::with_data_dir(&data_dir).expect("content store should open");
    (store, data_dir)
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
fn ignores_status_links_inside_non_post_regions() {
    let batch = batch(vec![element_with_root(
        "client-1",
        "section",
        None,
        "What's happening Trending item",
        Some("https://x.com/alice/status/12345?s=20"),
    )]);

    assert!(extract_items(&batch).is_empty());
}

#[test]
fn does_not_use_page_status_url_for_non_post_regions() {
    let batch = batch_with_url(
        "https://x.com/alice/status/12345",
        vec![element_with_root(
            "client-1",
            "section",
            None,
            "What's happening Trending item",
            None,
        )],
    );

    assert!(extract_items(&batch).is_empty());
}

#[test]
fn uses_page_status_url_for_post_like_regions() {
    let batch = batch_with_url(
        "https://x.com/alice/status/12345",
        vec![element("client-1", "Main post text", None)],
    );

    let extracted = extract_items(&batch);

    assert_eq!(extracted.len(), 1);
    assert_eq!(extracted[0].item.content_id.as_deref(), Some("12345"));
    assert_eq!(
        extracted[0].item.url.as_deref(),
        Some("https://x.com/alice/status/12345")
    );
}

#[test]
fn uses_page_status_url_for_only_one_post_like_region() {
    let batch = batch_with_url(
        "https://x.com/alice/status/12345",
        vec![
            element("client-1", "Main post text", None),
            element("client-2", "Reply text without its own status link", None),
        ],
    );

    let extracted = extract_items(&batch);

    assert_eq!(extracted.len(), 1);
    assert_eq!(extracted[0].item.client_id, "client-1");
    assert_eq!(extracted[0].item.content_id.as_deref(), Some("12345"));
}

#[test]
fn data_testid_tweet_root_is_post_region_evidence() {
    let batch = batch(vec![data_testid_tweet_element(
        "client-1",
        "Post text",
        Some("https://x.com/alice/status/12345?s=20"),
    )]);

    assert_eq!(extract_items(&batch).len(), 1);
}

#[test]
fn posts_with_text_go_to_codex_for_review() {
    assert!(should_ask_codex(&item("Just a normal post.", None)));
}

#[test]
fn url_only_posts_go_to_codex_for_review() {
    assert!(should_ask_codex(&item(
        "   ",
        Some("https://x.com/user/status/1")
    )));
}

#[test]
fn empty_posts_without_url_do_not_go_to_codex() {
    assert!(!should_ask_codex(&item("   ", None)));
}

#[test]
fn cached_decisions_reuse_summary_without_showing_it() {
    let cached_item = content_item("cached", "12345", "Timeline text");
    let current_item = content_item("current", "12345", "Detail page text with extra context");
    let ai_analyzer =
        AiAnalyzer::for_tests_with_x_summaries(&[(&cached_item, "cached summary", 0.82)]);

    let decisions =
        cached_decide_items(&[current_item], &ai_analyzer, &[]).expect("summary should be cached");

    assert_eq!(decisions.len(), 1);
    assert!(matches!(decisions[0].action, DecisionAction::Keep));
    assert_eq!(decisions[0].client_id, "current");
    assert_eq!(decisions[0].label, None);
}

#[test]
fn cached_decisions_return_none_when_required_summary_is_missing() {
    let current_item = content_item("current", "12345", "Needs a summary");
    let ai_analyzer = AiAnalyzer::for_tests_with_x_summaries(&[]);

    assert!(cached_decide_items(&[current_item], &ai_analyzer, &[]).is_none());
}

#[test]
fn rule_match_opinions_hide_posts() {
    let decision = reviewed_item_decision(AiOpinion {
        client_id: "client-1".into(),
        action: AiAction::Hide,
        opinion: "Matches low-value reaction rule".into(),
        confidence: 0.91,
        matched_rule_ids: vec!["x-engagement-bait-reaction".into()],
    });

    assert!(matches!(decision.action, DecisionAction::Hide));
    assert_eq!(decision.client_id, "client-1");
    assert_eq!(decision.label.as_deref(), Some("WebLayer: hidden by rule"));
    assert_eq!(
        decision.reason.as_deref(),
        Some("Matches low-value reaction rule")
    );
    assert_eq!(decision.confidence, Some(0.91));
    assert_eq!(
        decision.matched_rule_ids,
        vec!["x-engagement-bait-reaction".to_string()]
    );
}

#[test]
fn pending_commands_install_feedback_controls_for_all_posts() {
    let (content_store, data_dir) = content_store("pending-controls");
    let cached_item = content_item("cached", "12345", "Timeline text");
    let ai_analyzer =
        AiAnalyzer::for_tests_with_x_summaries(&[(&cached_item, "cached summary", 0.82)]);
    let batch = batch(vec![
        element(
            "client-1",
            "Cached post text has changed",
            Some("https://x.com/user/status/12345"),
        ),
        element(
            "client-2",
            "New post text",
            Some("https://x.com/user/status/67890"),
        ),
    ]);

    let commands = pending_dom_commands(&batch, &ai_analyzer, &content_store);

    assert_eq!(commands.len(), 2);
    assert!(commands.iter().all(|command| matches!(
        command.action,
        crate::core::DomCommandAction::InsertFeedbackControl
    )));
    assert_eq!(commands[0].target.client_id, "client-1");
    assert_eq!(commands[1].target.client_id, "client-2");
    assert!(commands[0]
        .feedback_context_id
        .as_deref()
        .is_some_and(|id| !id.is_empty()));
    assert!(commands[1]
        .feedback_context_id
        .as_deref()
        .is_some_and(|id| !id.is_empty()));

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn feedback_context_includes_at_most_twenty_active_rules() {
    let (content_store, data_dir) = content_store("feedback-context-rule-cap");
    let ai_analyzer = AiAnalyzer::for_tests_with_x_summaries(&[]);
    for index in 0..25 {
        content_store
            .x_create_rule(RuleCreateInput {
                id: Some(format!("x-test-rule-{index:02}")),
                status: Some("active".into()),
                priority: Some(100 + index as i64),
                title: format!("Test rule {index}"),
                instruction: format!("Hide test pattern {index}."),
                created_source: "test".into(),
                examples: RuleExamples::default(),
            })
            .expect("rule should create");
    }

    let commands = pending_dom_commands(
        &batch(vec![element(
            "client-1",
            "Post text",
            Some("https://x.com/user/status/12345"),
        )]),
        &ai_analyzer,
        &content_store,
    );
    let context_id = commands[0]
        .feedback_context_id
        .as_deref()
        .expect("feedback context ID should exist");
    let feedback_context = content_store
        .x_feedback_context(context_id)
        .expect("feedback context should load")
        .expect("feedback context should exist");

    assert_eq!(feedback_context.active_rules.len(), 20);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn thumbs_down_feedback_records_state_without_hiding_immediately() {
    let (content_store, data_dir) = content_store("thumbs-down-state");
    let batch = batch(vec![element(
        "client-1",
        "Post text",
        Some("https://x.com/user/status/12345"),
    )]);
    let ai_analyzer = AiAnalyzer::for_tests_with_x_summaries(&[]);
    let feedback_context_id = content_store
        .store_x_feedback_context(&FeedbackContext::default())
        .expect("feedback context should store");

    let commands = apply_feedback(
        &batch,
        FeedbackKind::ThumbsDown,
        "low quality",
        feedback_context_id.as_str(),
        &content_store,
    )
    .expect("feedback should apply");
    let pending_commands = pending_dom_commands(&batch, &ai_analyzer, &content_store);

    assert!(commands.is_empty());
    assert_eq!(pending_commands.len(), 1);
    assert_eq!(pending_commands[0].target.client_id, "client-1");
    assert!(matches!(
        pending_commands[0].action,
        crate::core::DomCommandAction::Hide
    ));
    assert_eq!(
        pending_commands[0].label.as_deref(),
        Some("WebLayer: hidden")
    );

    let _ = std::fs::remove_dir_all(data_dir);
}
