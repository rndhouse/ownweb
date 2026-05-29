use super::rules::{
    DEFAULT_NEW_RULE_PRIORITY, DEFAULT_RULE_ID, DEFAULT_RULE_INSTRUCTION, DEFAULT_RULE_PRIORITY,
    DEFAULT_RULE_SOURCE, DEFAULT_RULE_STATUS, DEFAULT_RULE_TITLE,
};
use super::*;
use crate::{
    core::{AnalysisBatch, ContentItem, FeedbackContext, FeedbackKind, FeedbackRuleContext},
    storage::{
        ContentAnnotationInput, ContentAnnotationQuery, ContentQuery, RuleCreateInput,
        RuleExamples, RuleQuery, RuleSetProposalAction, RuleSetProposalChange,
        RuleSetProposalCreateInput, RuleSetProposalQuery, RuleStatusInput, RuleSuggestionQuery,
        RuleUpdateInput, RuleValidationQuery, XDislikeQuery,
    },
};
use serde_json::{json, Value};
use std::path::PathBuf;

fn item(client_id: &str, post_id: Option<&str>, text: &str) -> ContentItem {
    ContentItem {
        client_id: client_id.into(),
        content_id: post_id.map(ToOwned::to_owned),
        url: post_id.map(|id| format!("https://x.com/user/status/{id}?utm=1")),
        author: Some("@user".into()),
        text: text.into(),
        captured_at: Some("2026-05-21T12:00:00.000Z".into()),
        kind: Some("post".into()),
        metadata: Value::Null,
    }
}

fn batch(source: &str, items: Vec<ContentItem>) -> AnalysisBatch {
    AnalysisBatch::new(source, items)
}

fn feedback_context() -> FeedbackContext {
    FeedbackContext::default()
}

fn temp_db_path(name: &str) -> PathBuf {
    let data_dir = std::env::temp_dir().join(format!(
        "weblayer-x-storage-test-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&data_dir);
    data_dir.join("x.com/db.sqlite")
}

#[test]
fn stores_x_posts_in_site_database() {
    let db_path = temp_db_path("stores-posts");
    let mut store = Store::open(&db_path).expect("store should open");

    store
        .record_batch(&batch(
            "x.com",
            vec![item("client-1", Some("123"), "hello")],
        ))
        .expect("batch should store");

    let text: String = store
        .connection
        .query_row(
            "SELECT text FROM tweets WHERE storage_key = 'x:id:123'",
            [],
            |row| row.get(0),
        )
        .expect("tweet should exist");
    assert_eq!(text, "hello");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn upserts_x_posts_by_post_id() {
    let db_path = temp_db_path("upserts-posts");
    let mut store = Store::open(&db_path).expect("store should open");

    store
        .record_batch(&batch(
            "x.com",
            vec![item("client-1", Some("123"), "first")],
        ))
        .expect("first batch should store");
    store
        .record_batch(&batch(
            "x.com",
            vec![item("client-2", Some("123"), "second")],
        ))
        .expect("second batch should store");

    let (text, seen_count, latest_client_id): (String, i64, String) = store
        .connection
        .query_row(
            "SELECT text, seen_count, latest_client_id FROM tweets WHERE storage_key = 'x:id:123'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("tweet should exist");

    assert_eq!(text, "second");
    assert_eq!(seen_count, 2);
    assert_eq!(latest_client_id, "client-2");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn content_stats_count_unique_posts_and_encounters() {
    let db_path = temp_db_path("content-stats");
    let mut store = Store::open(&db_path).expect("store should open");

    store
        .record_batch(&batch(
            "x.com",
            vec![
                item("client-1", Some("123"), "first"),
                item("client-2", Some("456"), "second"),
            ],
        ))
        .expect("first batch should store");
    store
        .record_batch(&batch(
            "x.com",
            vec![item("client-3", Some("123"), "first again")],
        ))
        .expect("second batch should store");

    let stats = store.content_stats().expect("stats should load");

    assert_eq!(stats.content_kind, "post");
    assert_eq!(stats.unique_items, 2);
    assert_eq!(stats.total_encounters, 3);
    assert_eq!(stats.items_with_stable_id, 2);
    assert!(stats.first_seen_at_unix_ms.is_some());
    assert!(stats.last_seen_at_unix_ms.is_some());

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn lists_content_by_recent_seen_time() {
    let db_path = temp_db_path("lists-content");
    let mut store = Store::open(&db_path).expect("store should open");

    store
        .record_batch(&batch(
            "x.com",
            vec![item("client-1", Some("123"), "first post")],
        ))
        .expect("first batch should store");
    std::thread::sleep(std::time::Duration::from_millis(1));
    store
        .record_batch(&batch(
            "x.com",
            vec![item("client-2", Some("456"), "second post")],
        ))
        .expect("second batch should store");

    let page = store
        .content(ContentQuery {
            search: None,
            limit: 100,
            offset: 0,
        })
        .expect("content should load");

    assert_eq!(page.total_matching, 2);
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].content_id.as_deref(), Some("456"));
    assert_eq!(page.items[0].content_kind, "post");
    assert_eq!(page.items[0].snippet, None);
    assert_eq!(page.items[1].content_id.as_deref(), Some("123"));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn searches_content_with_fts_index() {
    let db_path = temp_db_path("searches-content");
    let mut store = Store::open(&db_path).expect("store should open");

    store
        .record_batch(&batch(
            "x.com",
            vec![
                item("client-1", Some("123"), "Codex makes local search useful"),
                item("client-2", Some("456"), "unrelated post"),
            ],
        ))
        .expect("batch should store");

    let page = store
        .content(ContentQuery {
            search: Some("codex search".into()),
            limit: 100,
            offset: 0,
        })
        .expect("search should load");

    assert_eq!(page.total_matching, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].content_id.as_deref(), Some("123"));
    assert_eq!(page.items[0].text, "Codex makes local search useful");
    assert_eq!(
        page.items[0].snippet.as_deref(),
        Some("Codex makes local search useful")
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn search_index_is_rebuilt_for_existing_rows_on_open() {
    let db_path = temp_db_path("rebuilds-search");
    {
        let mut store = Store::open(&db_path).expect("store should open");
        store
            .record_batch(&batch(
                "x.com",
                vec![item("client-1", Some("123"), "persistent searchable text")],
            ))
            .expect("batch should store");
    }

    let store = Store::open(&db_path).expect("store should reopen");
    let page = store
        .content(ContentQuery {
            search: Some("persistent".into()),
            limit: 100,
            offset: 0,
        })
        .expect("search should load");

    assert_eq!(page.total_matching, 1);
    assert_eq!(page.items[0].content_id.as_deref(), Some("123"));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn records_feedback_rule_context_for_x_post() {
    let db_path = temp_db_path("records-feedback-context");
    let mut store = Store::open(&db_path).expect("store should open");
    let item = item("client-1", Some("123"), "hello");
    let feedback_context = FeedbackContext {
        active_rules: vec![FeedbackRuleContext {
            id: "x-low-value".into(),
            priority: 20,
            title: "Low value".into(),
            instruction: "Hide low-value posts.".into(),
            updated_at_unix_ms: 42,
            positive_examples: vec!["reply yes".into()],
            negative_examples: vec!["detailed notes".into()],
        }],
        decision: None,
    };
    let context_id = store
        .store_feedback_context(&feedback_context)
        .expect("feedback context should store");
    let loaded_context = store
        .feedback_context(&context_id)
        .expect("feedback context should load")
        .expect("feedback context should exist");

    store
        .record_feedback_with_context(
            &item,
            FeedbackKind::ThumbsDown,
            "low value",
            &feedback_context,
        )
        .expect("feedback should store");

    let stored_context: String = store
        .connection
        .query_row(
            "
            SELECT rule_context_json
            FROM tweet_feedback
            WHERE post_id = '123'
            ",
            [],
            |row| row.get(0),
        )
        .expect("feedback context should exist");
    let page = store
        .dislikes(XDislikeQuery {
            active: Some(true),
            limit: 10,
            offset: 0,
        })
        .expect("dislikes should load");

    assert_eq!(
        serde_json::from_str::<FeedbackContext>(&stored_context).expect("context should parse"),
        feedback_context
    );
    assert_eq!(loaded_context, feedback_context);
    assert_eq!(page.items[0].rule_context, feedback_context);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn records_thumbs_down_feedback_for_x_post() {
    let db_path = temp_db_path("records-feedback");
    let mut store = Store::open(&db_path).expect("store should open");
    let item = item("client-1", Some("123"), "hello");

    let recorded = store
        .record_feedback_with_context(&item, FeedbackKind::ThumbsDown, "", &feedback_context())
        .expect("feedback should store");

    let (storage_key, post_id, feedback_kind, reason, client_id): (
        String,
        String,
        String,
        String,
        String,
    ) = store
        .connection
        .query_row(
            "
            SELECT storage_key, post_id, feedback_kind, reason, client_id
            FROM tweet_feedback
            WHERE post_id = '123'
            ",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("feedback should exist");

    assert!(recorded);
    assert_eq!(storage_key, "x:id:123");
    assert_eq!(post_id, "123");
    assert_eq!(feedback_kind, "thumbsDown");
    assert_eq!(reason, "");
    assert_eq!(client_id, "client-1");
    assert_eq!(
        store
            .feedback_state(&item)
            .expect("state should load")
            .expect("state should exist"),
        super::super::XFeedbackState {
            active: true,
            reason: "".into(),
        }
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn updates_feedback_state_reason_for_x_post() {
    let db_path = temp_db_path("updates-feedback-reason");
    let mut store = Store::open(&db_path).expect("store should open");
    let item = item("client-1", Some("123"), "hello");

    store
        .record_feedback_with_context(&item, FeedbackKind::ThumbsDown, "", &feedback_context())
        .expect("feedback should store");
    store
        .record_feedback_with_context(
            &item,
            FeedbackKind::UpdateReason,
            "low information",
            &feedback_context(),
        )
        .expect("reason should store");

    let state = store
        .feedback_state(&item)
        .expect("state should load")
        .expect("state should exist");

    assert!(state.active);
    assert_eq!(state.reason, "low information");

    let latest_event: String = store
        .connection
        .query_row(
            "
            SELECT feedback_kind
            FROM tweet_feedback
            WHERE post_id = '123'
            ORDER BY id DESC
            LIMIT 1
            ",
            [],
            |row| row.get(0),
        )
        .expect("feedback event should exist");
    assert_eq!(latest_event, "updateReason");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn undo_feedback_deactivates_feedback_state_for_x_post() {
    let db_path = temp_db_path("undo-feedback-state");
    let mut store = Store::open(&db_path).expect("store should open");
    let item = item("client-1", Some("123"), "hello");

    store
        .record_feedback_with_context(
            &item,
            FeedbackKind::ThumbsDown,
            "low information",
            &feedback_context(),
        )
        .expect("feedback should store");
    store
        .record_feedback_with_context(&item, FeedbackKind::UndoThumbsDown, "", &feedback_context())
        .expect("undo should store");

    let state = store
        .feedback_state(&item)
        .expect("state should load")
        .expect("state should exist");

    assert!(!state.active);
    assert_eq!(state.reason, "");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn lists_x_dislikes_with_feedback_state_and_post_content() {
    let db_path = temp_db_path("lists-dislikes");
    let mut store = Store::open(&db_path).expect("store should open");
    let active_item = item("client-1", Some("123"), "active dislike text");
    let inactive_item = item("client-2", Some("456"), "inactive dislike text");

    store
        .record_batch(&batch(
            "x.com",
            vec![active_item.clone(), inactive_item.clone()],
        ))
        .expect("batch should store");
    store
        .record_feedback_with_context(
            &active_item,
            FeedbackKind::ThumbsDown,
            "low information",
            &feedback_context(),
        )
        .expect("active feedback should store");
    store
        .record_feedback_with_context(
            &inactive_item,
            FeedbackKind::ThumbsDown,
            "spam",
            &feedback_context(),
        )
        .expect("inactive feedback should store");
    store
        .record_feedback_with_context(
            &inactive_item,
            FeedbackKind::UndoThumbsDown,
            "",
            &feedback_context(),
        )
        .expect("undo should store");

    let active_page = store
        .dislikes(XDislikeQuery {
            active: Some(true),
            limit: 100,
            offset: 0,
        })
        .expect("active dislikes should load");
    let inactive_page = store
        .dislikes(XDislikeQuery {
            active: Some(false),
            limit: 100,
            offset: 0,
        })
        .expect("inactive dislikes should load");

    assert_eq!(active_page.total_matching, 1);
    assert_eq!(active_page.items.len(), 1);
    assert_eq!(active_page.items[0].post_id.as_deref(), Some("123"));
    assert_eq!(active_page.items[0].text, "active dislike text");
    assert_eq!(active_page.items[0].reason, "low information");
    assert!(active_page.items[0].active);
    assert_eq!(active_page.items[0].seen_count, Some(1));

    assert_eq!(inactive_page.total_matching, 1);
    assert_eq!(inactive_page.items.len(), 1);
    assert_eq!(inactive_page.items[0].post_id.as_deref(), Some("456"));
    assert!(!inactive_page.items[0].active);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn upserts_content_annotations_by_identity() {
    let db_path = temp_db_path("upserts-annotations");
    let mut store = Store::open(&db_path).expect("store should open");

    let first = store
        .upsert_content_annotation(ContentAnnotationInput {
            storage_key: "x:id:123".into(),
            content_kind: "post".into(),
            annotation_type: "tag".into(),
            key: "topics".into(),
            value: json!(["ai"]),
            confidence: Some(0.4),
            source: "agent:test".into(),
        })
        .expect("first annotation should store");
    let second = store
        .upsert_content_annotation(ContentAnnotationInput {
            storage_key: "x:id:123".into(),
            content_kind: "post".into(),
            annotation_type: "tag".into(),
            key: "topics".into(),
            value: json!(["ai", "coding"]),
            confidence: Some(0.9),
            source: "agent:test".into(),
        })
        .expect("second annotation should update");

    let count: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM content_annotations", [], |row| {
            row.get(0)
        })
        .expect("annotation count should load");

    assert_eq!(first.id, second.id);
    assert_eq!(count, 1);
    assert_eq!(second.value, json!(["ai", "coding"]));
    assert_eq!(second.confidence, Some(0.9));
    assert_eq!(second.created_at_unix_ms, first.created_at_unix_ms);
    assert!(second.updated_at_unix_ms >= second.created_at_unix_ms);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn lists_content_annotations_for_storage_key_or_content_id() {
    let db_path = temp_db_path("lists-annotations");
    let mut store = Store::open(&db_path).expect("store should open");

    store
        .upsert_content_annotation(ContentAnnotationInput {
            storage_key: "x:id:123".into(),
            content_kind: "post".into(),
            annotation_type: "note".into(),
            key: "summary".into(),
            value: json!("This post is about local AI tooling."),
            confidence: Some(0.8),
            source: "agent:test".into(),
        })
        .expect("note should store");
    store
        .upsert_content_annotation(ContentAnnotationInput {
            storage_key: "x:id:123".into(),
            content_kind: "post".into(),
            annotation_type: "tag".into(),
            key: "topics".into(),
            value: json!(["local-ai", "tools"]),
            confidence: None,
            source: "agent:test".into(),
        })
        .expect("tags should store");
    store
        .upsert_content_annotation(ContentAnnotationInput {
            storage_key: "x:id:456".into(),
            content_kind: "post".into(),
            annotation_type: "note".into(),
            key: "summary".into(),
            value: json!("Different post."),
            confidence: None,
            source: "agent:test".into(),
        })
        .expect("other note should store");

    let page = store
        .content_annotations(ContentAnnotationQuery {
            storage_key: None,
            content_id: Some("123".into()),
            content_kind: None,
            annotation_type: None,
            key: None,
            source: Some("agent:test".into()),
            limit: 100,
            offset: 0,
        })
        .expect("annotations should load");
    let note_page = store
        .content_annotations(ContentAnnotationQuery {
            storage_key: Some("x:id:123".into()),
            content_id: None,
            content_kind: None,
            annotation_type: Some("note".into()),
            key: Some("summary".into()),
            source: None,
            limit: 100,
            offset: 0,
        })
        .expect("filtered annotations should load");

    assert_eq!(page.total_matching, 2);
    assert_eq!(page.items.len(), 2);
    assert!(page.items.iter().all(|item| item.storage_key == "x:id:123"));
    assert_eq!(note_page.total_matching, 1);
    assert_eq!(note_page.items[0].annotation_type, "note");
    assert_eq!(note_page.items[0].key, "summary");
    assert_eq!(
        note_page.items[0].value,
        json!("This post is about local AI tooling.")
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn opens_with_default_active_content_rule() {
    let db_path = temp_db_path("opens-with-default-rule");
    let store = Store::open(&db_path).expect("store should open");

    let rules = store
        .rules(RuleQuery {
            status: Some("active".into()),
            limit: 100,
            offset: 0,
        })
        .expect("rules should load");

    assert_eq!(rules.total_matching, 1);
    assert_eq!(rules.items.len(), 1);
    assert_eq!(rules.items[0].id, DEFAULT_RULE_ID);
    assert_eq!(rules.items[0].site, SITE_DIR);
    assert_eq!(rules.items[0].status, DEFAULT_RULE_STATUS);
    assert_eq!(rules.items[0].priority, DEFAULT_RULE_PRIORITY);
    assert_eq!(rules.items[0].title, DEFAULT_RULE_TITLE);
    assert_eq!(rules.items[0].instruction, DEFAULT_RULE_INSTRUCTION);
    assert_eq!(rules.items[0].created_source, DEFAULT_RULE_SOURCE);
    assert!(rules.items[0].examples.positive.is_empty());
    assert!(rules.items[0].examples.negative.is_empty());

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn creates_rules_as_drafts_with_audit_history() {
    let db_path = temp_db_path("creates-rule");
    let mut store = Store::open(&db_path).expect("store should open");

    let rule = store
        .create_rule(RuleCreateInput {
            id: Some("x-ai-slop".into()),
            status: None,
            priority: None,
            title: "AI slop".into(),
            instruction: "Hide generic AI engagement bait.".into(),
            created_source: "user".into(),
            examples: RuleExamples {
                positive: vec!["I asked ChatGPT to write this viral thread".into()],
                negative: vec!["Detailed local AI implementation notes".into()],
            },
        })
        .expect("rule should be created");
    let detail = store
        .rule_detail("x-ai-slop")
        .expect("rule detail should load")
        .expect("rule should exist");

    assert_eq!(rule.status, "draft");
    assert_eq!(rule.priority, DEFAULT_NEW_RULE_PRIORITY);
    assert_eq!(detail.rule.id, "x-ai-slop");
    assert_eq!(detail.audit.len(), 1);
    assert_eq!(detail.audit[0].event_kind, "created");
    assert_eq!(detail.audit[0].source, "user");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn updates_rule_status_priority_and_examples() {
    let db_path = temp_db_path("updates-rule");
    let mut store = Store::open(&db_path).expect("store should open");

    store
        .create_rule(RuleCreateInput {
            id: Some("x-low-value".into()),
            status: None,
            priority: None,
            title: "Low value".into(),
            instruction: "Hide low value posts.".into(),
            created_source: "user".into(),
            examples: RuleExamples::default(),
        })
        .expect("rule should be created");
    let updated = store
        .update_rule(RuleUpdateInput {
            id: "x-low-value".into(),
            status: Some("active".into()),
            priority: Some(25),
            title: Some("Low-value engagement".into()),
            instruction: None,
            source: "user".into(),
            positive_examples: Some(vec!["reply with yes if you agree".into()]),
            negative_examples: None,
        })
        .expect("rule should update")
        .expect("rule should exist");
    let disabled = store
        .update_rule_status(RuleStatusInput {
            id: "x-low-value".into(),
            status: "disabled".into(),
            source: "user".into(),
        })
        .expect("status should update")
        .expect("rule should exist");
    let detail = store
        .rule_detail("x-low-value")
        .expect("rule detail should load")
        .expect("rule should exist");

    assert_eq!(updated.status, "active");
    assert_eq!(updated.priority, 25);
    assert_eq!(updated.title, "Low-value engagement");
    assert_eq!(
        updated.examples.positive,
        vec!["reply with yes if you agree".to_string()]
    );
    assert_eq!(disabled.status, "disabled");
    assert_eq!(detail.audit.len(), 3);
    assert_eq!(detail.audit[0].event_kind, "disabled");
    assert_eq!(detail.audit[1].event_kind, "updated");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn validates_rules_against_stored_content() {
    let db_path = temp_db_path("validates-rule");
    let mut store = Store::open(&db_path).expect("store should open");

    store
        .record_batch(&batch(
            "x.com",
            vec![
                item(
                    "client-1",
                    Some("123"),
                    "Reply with yes if you agree with this viral engagement bait",
                ),
                item("client-2", Some("456"), "Detailed notes about local search"),
            ],
        ))
        .expect("content should store");
    store
        .create_rule(RuleCreateInput {
            id: Some("x-engagement".into()),
            status: Some("active".into()),
            priority: Some(10),
            title: "Engagement bait".into(),
            instruction: "Hide engagement bait posts.".into(),
            created_source: "user".into(),
            examples: RuleExamples {
                positive: vec!["reply with yes if you agree".into()],
                negative: vec!["Detailed notes about local search".into()],
            },
        })
        .expect("rule should be created");

    let page = store
        .validate_rule(
            "x-engagement",
            RuleValidationQuery {
                limit: 10,
                offset: 0,
            },
        )
        .expect("validation should run")
        .expect("rule should exist");

    assert_eq!(page.total_stored, 2);
    assert_eq!(page.total_matching, 1);
    assert_eq!(page.items[0].content.content_id.as_deref(), Some("123"));
    assert!(page.items[0]
        .matched_examples
        .contains(&"reply with yes if you agree".to_string()));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn suggests_draft_rules_from_active_feedback_reasons() {
    let db_path = temp_db_path("suggests-rule");
    let mut store = Store::open(&db_path).expect("store should open");
    let first = item("client-1", Some("123"), "Engagement bait example one");
    let second = item("client-2", Some("456"), "Engagement bait example two");
    let ignored = item("client-3", Some("789"), "Different reason example");

    store
        .record_batch(&batch(
            "x.com",
            vec![first.clone(), second.clone(), ignored.clone()],
        ))
        .expect("content should store");
    store
        .record_feedback_with_context(
            &first,
            FeedbackKind::ThumbsDown,
            "engagement bait",
            &feedback_context(),
        )
        .expect("feedback should store");
    store
        .record_feedback_with_context(
            &second,
            FeedbackKind::ThumbsDown,
            "engagement bait",
            &feedback_context(),
        )
        .expect("feedback should store");
    store
        .record_feedback_with_context(
            &ignored,
            FeedbackKind::ThumbsDown,
            "spam",
            &feedback_context(),
        )
        .expect("feedback should store");

    let page = store
        .rule_suggestions(RuleSuggestionQuery {
            min_feedback: 2,
            limit: 10,
            offset: 0,
        })
        .expect("suggestions should load");

    assert_eq!(page.total_matching, 1);
    assert_eq!(page.items[0].status, "draft");
    assert_eq!(page.items[0].source, "feedback");
    assert_eq!(page.items[0].feedback_count, 2);
    assert_eq!(page.items[0].reasons, vec!["engagement bait".to_string()]);
    assert_eq!(page.items[0].examples.positive.len(), 2);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn stores_rule_set_proposals_for_review() {
    let db_path = temp_db_path("stores-rule-proposal");
    let mut store = Store::open(&db_path).expect("store should open");

    let proposal = store
        .create_rule_set_proposal(RuleSetProposalCreateInput {
            source: "agent:test".into(),
            feedback_count: 2,
            active_rule_count: 1,
            changes: vec![RuleSetProposalChange {
                action: RuleSetProposalAction::CreateRule,
                rule_id: Some("x-feedback-engagement".into()),
                status: Some("draft".into()),
                priority: Some(100),
                title: Some("Engagement bait".into()),
                instruction: Some("Hide engagement bait posts.".into()),
                rationale: "Two feedback examples share the same theme.".into(),
                evidence_storage_keys: vec!["x:id:123".into(), "x:id:123".into()],
                examples: RuleExamples {
                    positive: vec!["reply yes if you agree".into()],
                    negative: Vec::new(),
                },
            }],
        })
        .expect("proposal should store");

    let loaded = store
        .rule_set_proposal(&proposal.id)
        .expect("proposal should load")
        .expect("proposal should exist");
    let page = store
        .rule_set_proposals(RuleSetProposalQuery {
            status: Some("pending".into()),
            limit: 10,
            offset: 0,
        })
        .expect("proposals should list");

    assert_eq!(loaded.id, proposal.id);
    assert_eq!(loaded.status, "pending");
    assert_eq!(loaded.source, "agent:test");
    assert_eq!(loaded.changes[0].action, RuleSetProposalAction::CreateRule);
    assert_eq!(loaded.changes[0].evidence_storage_keys, vec!["x:id:123"]);
    assert_eq!(page.total_matching, 1);
    assert_eq!(page.items[0].id, proposal.id);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap().parent().unwrap());
}

#[test]
fn fallback_storage_key_uses_normalized_content() {
    let first = item("client-1", None, "hello   world");
    let second = item("client-2", None, "hello world");

    assert_eq!(
        storage_key(&first, None, &normalize_text(&first.text)),
        storage_key(&second, None, &normalize_text(&second.text))
    );
}

#[test]
fn status_id_can_be_extracted_from_url() {
    let mut item = item("client-1", None, "hello");
    item.url = Some("https://x.com/user/status/98765?s=20".into());

    assert_eq!(stable_post_id(&item).as_deref(), Some("98765"));
}
