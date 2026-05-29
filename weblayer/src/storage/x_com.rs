use crate::{
    core::{AnalysisBatch, ContentItem, FeedbackKind},
    storage::{
        ContentAnnotation, ContentAnnotationInput, ContentAnnotationPage, ContentAnnotationQuery,
        ContentPage, ContentQuery, ContentRule, ContentStats, RuleAuditEvent, RuleCreateInput,
        RuleDetail, RuleExamples, RulePage, RuleQuery, RuleStatusInput, RuleSuggestion,
        RuleSuggestionPage, RuleSuggestionQuery, RuleUpdateInput, RuleValidationMatch,
        RuleValidationPage, RuleValidationQuery, StoredContentItem, XDislikePage, XDislikeQuery,
        XDislikedPost,
    },
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::debug;

pub(super) const SITE_DIR: &str = "x.com";
const DEFAULT_RULE_ID: &str = "x-engagement-bait-reaction";
const DEFAULT_RULE_STATUS: &str = "active";
const DEFAULT_RULE_PRIORITY: i64 = 50;
const DEFAULT_RULE_TITLE: &str = "Engagement bait reaction posts";
const DEFAULT_RULE_INSTRUCTION: &str = "Downrank engagement bait, dunking, or 'look at this absurd thing' posts where the main content is a reaction to a video, image, or quote rather than a substantive claim.";
const DEFAULT_RULE_SOURCE: &str = "seed";
const DEFAULT_NEW_RULE_STATUS: &str = "draft";
const DEFAULT_NEW_RULE_PRIORITY: i64 = 100;
const RULE_AUDIT_LIMIT: usize = 50;

pub(super) struct Store {
    connection: Connection,
}

impl Store {
    pub(super) fn open(path: &Path) -> super::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS tweets (
                storage_key TEXT PRIMARY KEY,
                post_id TEXT,
                url TEXT,
                author_handle TEXT,
                text TEXT NOT NULL,
                normalized_text TEXT NOT NULL,
                text_hash TEXT NOT NULL,
                first_seen_at_unix_ms INTEGER NOT NULL,
                last_seen_at_unix_ms INTEGER NOT NULL,
                seen_count INTEGER NOT NULL,
                latest_client_id TEXT NOT NULL,
                latest_captured_at TEXT,
                latest_payload_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS tweets_post_id_idx
                ON tweets(post_id);
            CREATE INDEX IF NOT EXISTS tweets_author_handle_idx
                ON tweets(author_handle);
            CREATE INDEX IF NOT EXISTS tweets_last_seen_at_idx
                ON tweets(last_seen_at_unix_ms);

            CREATE TABLE IF NOT EXISTS tweet_feedback (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                storage_key TEXT NOT NULL,
                post_id TEXT,
                feedback_kind TEXT NOT NULL,
                reason TEXT NOT NULL,
                created_at_unix_ms INTEGER NOT NULL,
                client_id TEXT NOT NULL,
                url TEXT,
                author_handle TEXT,
                captured_at TEXT,
                payload_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS tweet_feedback_storage_key_idx
                ON tweet_feedback(storage_key);
            CREATE INDEX IF NOT EXISTS tweet_feedback_post_id_idx
                ON tweet_feedback(post_id);
            CREATE INDEX IF NOT EXISTS tweet_feedback_created_at_idx
                ON tweet_feedback(created_at_unix_ms);

            CREATE TABLE IF NOT EXISTS tweet_feedback_state (
                storage_key TEXT PRIMARY KEY,
                post_id TEXT,
                active INTEGER NOT NULL,
                feedback_kind TEXT NOT NULL,
                reason TEXT NOT NULL,
                created_at_unix_ms INTEGER NOT NULL,
                updated_at_unix_ms INTEGER NOT NULL,
                latest_client_id TEXT NOT NULL,
                url TEXT,
                author_handle TEXT,
                latest_captured_at TEXT,
                latest_payload_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS tweet_feedback_state_active_idx
                ON tweet_feedback_state(active);
            CREATE INDEX IF NOT EXISTS tweet_feedback_state_post_id_idx
                ON tweet_feedback_state(post_id);

            CREATE TABLE IF NOT EXISTS content_rules (
                id TEXT PRIMARY KEY,
                site TEXT NOT NULL,
                status TEXT NOT NULL,
                priority INTEGER NOT NULL,
                title TEXT NOT NULL,
                instruction TEXT NOT NULL,
                created_source TEXT NOT NULL,
                created_at_unix_ms INTEGER NOT NULL,
                updated_at_unix_ms INTEGER NOT NULL,
                examples_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS content_rules_status_priority_idx
                ON content_rules(status, priority);

            CREATE TABLE IF NOT EXISTS content_rule_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                rule_id TEXT NOT NULL,
                event_kind TEXT NOT NULL,
                source TEXT NOT NULL,
                created_at_unix_ms INTEGER NOT NULL,
                snapshot_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS content_rule_events_rule_time_idx
                ON content_rule_events(rule_id, created_at_unix_ms);

            CREATE TABLE IF NOT EXISTS content_annotations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                storage_key TEXT NOT NULL,
                content_kind TEXT NOT NULL,
                annotation_type TEXT NOT NULL,
                annotation_key TEXT NOT NULL,
                value_json TEXT NOT NULL,
                value_text TEXT NOT NULL,
                confidence REAL,
                source TEXT NOT NULL,
                created_at_unix_ms INTEGER NOT NULL,
                updated_at_unix_ms INTEGER NOT NULL,
                UNIQUE(storage_key, annotation_type, annotation_key, source)
            );

            CREATE INDEX IF NOT EXISTS content_annotations_storage_key_idx
                ON content_annotations(storage_key);
            CREATE INDEX IF NOT EXISTS content_annotations_type_key_idx
                ON content_annotations(annotation_type, annotation_key);
            CREATE INDEX IF NOT EXISTS content_annotations_source_idx
                ON content_annotations(source);
            CREATE INDEX IF NOT EXISTS content_annotations_updated_at_idx
                ON content_annotations(updated_at_unix_ms);
            ",
        )?;
        migrate_search(&connection)?;
        seed_default_rules(&connection)?;

        Ok(Self { connection })
    }

    pub(super) fn record_batch(&mut self, batch: &AnalysisBatch) -> super::Result<()> {
        let seen_at = now_unix_ms();
        let transaction = self.connection.transaction()?;
        let mut stored_count = 0usize;
        let mut skipped_count = 0usize;

        for item in &batch.items {
            let Some(record) = StoredTweet::from_item(batch.source.as_str(), item, seen_at)? else {
                skipped_count += 1;
                continue;
            };

            transaction.execute(
                "
                INSERT INTO tweets (
                    storage_key,
                    post_id,
                    url,
                    author_handle,
                    text,
                    normalized_text,
                    text_hash,
                    first_seen_at_unix_ms,
                    last_seen_at_unix_ms,
                    seen_count,
                    latest_client_id,
                    latest_captured_at,
                    latest_payload_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 1, ?9, ?10, ?11)
                ON CONFLICT(storage_key) DO UPDATE SET
                    post_id = COALESCE(excluded.post_id, tweets.post_id),
                    url = COALESCE(excluded.url, tweets.url),
                    author_handle = COALESCE(excluded.author_handle, tweets.author_handle),
                    text = excluded.text,
                    normalized_text = excluded.normalized_text,
                    text_hash = excluded.text_hash,
                    last_seen_at_unix_ms = excluded.last_seen_at_unix_ms,
                    seen_count = tweets.seen_count + 1,
                    latest_client_id = excluded.latest_client_id,
                    latest_captured_at = excluded.latest_captured_at,
                    latest_payload_json = excluded.latest_payload_json
                ",
                params![
                    record.storage_key,
                    record.post_id,
                    record.url,
                    record.author_handle,
                    record.text,
                    record.normalized_text,
                    record.text_hash,
                    record.seen_at_unix_ms,
                    record.client_id,
                    record.captured_at,
                    record.payload_json,
                ],
            )?;
            stored_count += 1;
        }

        transaction.commit()?;

        debug!(
            target: "weblayer_daemon::storage::x_com",
            source = batch.source.as_str(),
            stored_count,
            skipped_count,
            "stored X content batch"
        );

        Ok(())
    }

    pub(super) fn record_feedback(
        &mut self,
        item: &ContentItem,
        feedback: FeedbackKind,
        reason: &str,
    ) -> super::Result<bool> {
        let created_at_unix_ms = now_unix_ms();
        let Some(record) =
            StoredTweetFeedback::from_item(item, feedback, reason, created_at_unix_ms)?
        else {
            debug!(
                target: "weblayer_daemon::storage::x_com",
                client_id = item.client_id.as_str(),
                "skipped X feedback without stable storage key"
            );
            return Ok(false);
        };

        let transaction = self.connection.transaction()?;
        transaction.execute(
            "
            INSERT INTO tweet_feedback (
                storage_key,
                post_id,
                feedback_kind,
                reason,
                created_at_unix_ms,
                client_id,
                url,
                author_handle,
                captured_at,
                payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ",
            params![
                record.storage_key,
                record.post_id,
                record.feedback_kind,
                record.reason,
                record.created_at_unix_ms,
                record.client_id,
                record.url,
                record.author_handle,
                record.captured_at,
                record.payload_json,
            ],
        )?;

        transaction.execute(
            "
            INSERT INTO tweet_feedback_state (
                storage_key,
                post_id,
                active,
                feedback_kind,
                reason,
                created_at_unix_ms,
                updated_at_unix_ms,
                latest_client_id,
                url,
                author_handle,
                latest_captured_at,
                latest_payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(storage_key) DO UPDATE SET
                post_id = COALESCE(excluded.post_id, tweet_feedback_state.post_id),
                active = excluded.active,
                feedback_kind = excluded.feedback_kind,
                reason = excluded.reason,
                updated_at_unix_ms = excluded.updated_at_unix_ms,
                latest_client_id = excluded.latest_client_id,
                url = COALESCE(excluded.url, tweet_feedback_state.url),
                author_handle = COALESCE(excluded.author_handle, tweet_feedback_state.author_handle),
                latest_captured_at = excluded.latest_captured_at,
                latest_payload_json = excluded.latest_payload_json
            ",
            params![
                record.storage_key,
                record.post_id,
                feedback_state_active(feedback),
                record.feedback_kind,
                record.reason,
                record.created_at_unix_ms,
                record.client_id,
                record.url,
                record.author_handle,
                record.captured_at,
                record.payload_json,
            ],
        )?;
        transaction.commit()?;

        debug!(
            target: "weblayer_daemon::storage::x_com",
            storage_key = record.storage_key.as_str(),
            post_id = record.post_id.as_deref(),
            feedback_kind = record.feedback_kind.as_str(),
            "stored X feedback"
        );

        Ok(true)
    }

    pub(super) fn feedback_state(
        &self,
        item: &ContentItem,
    ) -> super::Result<Option<super::XFeedbackState>> {
        let post_id = stable_post_id(item);
        let normalized_text = normalize_text(&item.text);
        let Some(storage_key) = storage_key(item, post_id.as_deref(), &normalized_text) else {
            return Ok(None);
        };

        let state = self
            .connection
            .query_row(
                "
                SELECT active, reason
                FROM tweet_feedback_state
                WHERE storage_key = ?1
                ",
                [storage_key],
                |row| {
                    let active: i64 = row.get(0)?;
                    let reason: String = row.get(1)?;
                    Ok(super::XFeedbackState {
                        active: active != 0,
                        reason,
                    })
                },
            )
            .optional()?;

        Ok(state)
    }

    pub(super) fn dislikes(&self, query: XDislikeQuery) -> super::Result<XDislikePage> {
        let active = query.active.map(bool_to_sqlite_int);
        let limit = sqlite_limit(query.limit);
        let offset = sqlite_limit(query.offset);
        let total_matching = self.connection.query_row(
            "
            SELECT COUNT(*)
            FROM tweet_feedback_state
            WHERE (?1 IS NULL OR active = ?1)
            ",
            [active],
            |row| row.get::<_, i64>(0),
        )?;

        let mut statement = self.connection.prepare(
            "
            SELECT
                state.storage_key,
                state.post_id,
                COALESCE(state.url, tweets.url) AS url,
                COALESCE(state.author_handle, tweets.author_handle) AS author,
                COALESCE(tweets.text, '') AS text,
                state.reason,
                state.active,
                state.feedback_kind,
                state.created_at_unix_ms,
                state.updated_at_unix_ms,
                tweets.first_seen_at_unix_ms,
                tweets.last_seen_at_unix_ms,
                tweets.seen_count,
                COALESCE(state.latest_captured_at, tweets.latest_captured_at) AS latest_captured_at
            FROM tweet_feedback_state state
            LEFT JOIN tweets ON tweets.storage_key = state.storage_key
            WHERE (?1 IS NULL OR state.active = ?1)
            ORDER BY state.updated_at_unix_ms DESC, state.storage_key ASC
            LIMIT ?2 OFFSET ?3
            ",
        )?;
        let items = statement
            .query_map(params![active, limit, offset], |row| {
                let active: i64 = row.get(6)?;
                Ok(XDislikedPost {
                    storage_key: row.get(0)?,
                    post_id: row.get(1)?,
                    url: row.get(2)?,
                    author: row.get(3)?,
                    text: row.get(4)?,
                    reason: row.get(5)?,
                    active: active != 0,
                    feedback_kind: row.get(7)?,
                    disliked_at_unix_ms: row.get(8)?,
                    updated_at_unix_ms: row.get(9)?,
                    first_seen_at_unix_ms: row.get(10)?,
                    last_seen_at_unix_ms: row.get(11)?,
                    seen_count: row.get(12)?,
                    latest_captured_at: row.get(13)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(XDislikePage {
            total_matching: total_matching.max(0) as usize,
            limit: limit as usize,
            offset: offset as usize,
            items,
        })
    }

    pub(super) fn rules(&self, query: RuleQuery) -> super::Result<RulePage> {
        let status = clean_optional(query.status.as_deref());
        let limit = sqlite_limit(query.limit);
        let offset = sqlite_limit(query.offset);
        let total_matching = self.connection.query_row(
            "
            SELECT COUNT(*)
            FROM content_rules
            WHERE (?1 IS NULL OR status = ?1)
            ",
            [status.as_deref()],
            |row| row.get::<_, i64>(0),
        )?;

        let mut statement = self.connection.prepare(
            "
            SELECT
                id,
                site,
                status,
                priority,
                title,
                instruction,
                created_source,
                created_at_unix_ms,
                updated_at_unix_ms,
                examples_json
            FROM content_rules
            WHERE (?1 IS NULL OR status = ?1)
            ORDER BY priority ASC, id ASC
            LIMIT ?2 OFFSET ?3
            ",
        )?;
        let items = statement
            .query_map(params![status.as_deref(), limit, offset], |row| {
                let examples_json: String = row.get(9)?;
                let examples = serde_json::from_str(&examples_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        9,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;

                Ok(ContentRule {
                    id: row.get(0)?,
                    site: row.get(1)?,
                    status: row.get(2)?,
                    priority: row.get(3)?,
                    title: row.get(4)?,
                    instruction: row.get(5)?,
                    created_source: row.get(6)?,
                    created_at_unix_ms: row.get(7)?,
                    updated_at_unix_ms: row.get(8)?,
                    examples,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(RulePage {
            total_matching: total_matching.max(0) as usize,
            limit: limit as usize,
            offset: offset as usize,
            items,
        })
    }

    pub(super) fn rule_detail(&self, id: &str) -> super::Result<Option<RuleDetail>> {
        let Some(id) = clean_rule_id_for_lookup(id) else {
            return Ok(None);
        };
        let Some(rule) = self.rule(&id)? else {
            return Ok(None);
        };
        let audit = self.rule_audit(&id, RULE_AUDIT_LIMIT)?;

        Ok(Some(RuleDetail { rule, audit }))
    }

    pub(super) fn create_rule(&mut self, input: RuleCreateInput) -> super::Result<ContentRule> {
        let now = now_unix_ms();
        let title = required_clean_text(&input.title, "title")?;
        let instruction = required_clean_text(&input.instruction, "instruction")?;
        let created_source = required_clean_text(&input.created_source, "source")?;
        let status = clean_rule_status(input.status.as_deref().unwrap_or(DEFAULT_NEW_RULE_STATUS))?;
        let priority = input.priority.unwrap_or(DEFAULT_NEW_RULE_PRIORITY);
        let examples = clean_examples(input.examples);
        let id = match input.id {
            Some(id) => clean_rule_id(&id)?,
            None => generated_rule_id(&title, &instruction, now),
        };

        if self.rule(&id)?.is_some() {
            return Err(super::StorageError::InvalidInput(format!(
                "rule id already exists: {id}"
            )));
        }

        let examples_json = serde_json::to_string(&examples)?;
        self.connection.execute(
            "
            INSERT INTO content_rules (
                id,
                site,
                status,
                priority,
                title,
                instruction,
                created_source,
                created_at_unix_ms,
                updated_at_unix_ms,
                examples_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)
            ",
            params![
                id,
                SITE_DIR,
                status,
                priority,
                title,
                instruction,
                created_source,
                now,
                examples_json,
            ],
        )?;

        let rule = self
            .rule(&id)?
            .expect("created rule should be readable immediately");
        self.record_rule_event(&rule, "created", &rule.created_source, now)?;

        Ok(rule)
    }

    pub(super) fn update_rule(
        &mut self,
        input: RuleUpdateInput,
    ) -> super::Result<Option<ContentRule>> {
        let Some(id) = clean_rule_id_for_lookup(&input.id) else {
            return Ok(None);
        };
        let Some(existing) = self.rule(&id)? else {
            return Ok(None);
        };

        let source = required_clean_text(&input.source, "source")?;
        let status = match input.status {
            Some(status) => clean_rule_status(&status)?,
            None => existing.status,
        };
        let priority = input.priority.unwrap_or(existing.priority);
        let title = match input.title {
            Some(title) => required_clean_text(&title, "title")?,
            None => existing.title,
        };
        let instruction = match input.instruction {
            Some(instruction) => required_clean_text(&instruction, "instruction")?,
            None => existing.instruction,
        };
        let mut examples = existing.examples;
        if let Some(positive_examples) = input.positive_examples {
            examples.positive = clean_example_list(positive_examples);
        }
        if let Some(negative_examples) = input.negative_examples {
            examples.negative = clean_example_list(negative_examples);
        }
        let examples_json = serde_json::to_string(&examples)?;
        let now = now_unix_ms();

        self.connection.execute(
            "
            UPDATE content_rules
            SET
                status = ?2,
                priority = ?3,
                title = ?4,
                instruction = ?5,
                updated_at_unix_ms = ?6,
                examples_json = ?7
            WHERE id = ?1
            ",
            params![id, status, priority, title, instruction, now, examples_json],
        )?;

        let rule = self
            .rule(&id)?
            .expect("updated rule should be readable immediately");
        self.record_rule_event(&rule, "updated", &source, now)?;

        Ok(Some(rule))
    }

    pub(super) fn update_rule_status(
        &mut self,
        input: RuleStatusInput,
    ) -> super::Result<Option<ContentRule>> {
        let Some(id) = clean_rule_id_for_lookup(&input.id) else {
            return Ok(None);
        };
        if self.rule(&id)?.is_none() {
            return Ok(None);
        }

        let status = clean_rule_status(&input.status)?;
        let source = required_clean_text(&input.source, "source")?;
        let now = now_unix_ms();
        self.connection.execute(
            "
            UPDATE content_rules
            SET status = ?2, updated_at_unix_ms = ?3
            WHERE id = ?1
            ",
            params![id, status, now],
        )?;

        let rule = self
            .rule(&id)?
            .expect("status-updated rule should be readable immediately");
        self.record_rule_event(&rule, rule_status_event_kind(&rule.status), &source, now)?;

        Ok(Some(rule))
    }

    pub(super) fn validate_rule(
        &self,
        id: &str,
        query: RuleValidationQuery,
    ) -> super::Result<Option<RuleValidationPage>> {
        let Some(id) = clean_rule_id_for_lookup(id) else {
            return Ok(None);
        };
        let Some(rule) = self.rule(&id)? else {
            return Ok(None);
        };

        let matcher = RuleMatcher::new(&rule);
        let total_stored = self
            .connection
            .query_row("SELECT COUNT(*) FROM tweets", [], |row| {
                row.get::<_, i64>(0)
            })?;
        let mut statement = self.connection.prepare(
            "
            SELECT
                storage_key,
                post_id,
                url,
                author_handle,
                text,
                first_seen_at_unix_ms,
                last_seen_at_unix_ms,
                seen_count,
                latest_captured_at
            FROM tweets
            ORDER BY last_seen_at_unix_ms DESC, storage_key ASC
            ",
        )?;
        let mut matches = Vec::new();
        let rows = statement.query_map([], |row| content_item_from_row(row, None))?;
        for row in rows {
            let content = row?;
            if let Some(rule_match) = matcher.evaluate(content) {
                matches.push(rule_match);
            }
        }

        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| {
                    right
                        .content
                        .last_seen_at_unix_ms
                        .cmp(&left.content.last_seen_at_unix_ms)
                })
                .then_with(|| left.content.storage_key.cmp(&right.content.storage_key))
        });

        let limit = query.limit.min(i64::MAX as usize);
        let offset = query.offset.min(i64::MAX as usize);
        let total_matching = matches.len();
        let items = matches.into_iter().skip(offset).take(limit).collect();

        Ok(Some(RuleValidationPage {
            rule,
            total_stored: total_stored.max(0) as usize,
            total_matching,
            limit,
            offset,
            items,
        }))
    }

    pub(super) fn rule_suggestions(
        &self,
        query: RuleSuggestionQuery,
    ) -> super::Result<RuleSuggestionPage> {
        let min_feedback = query.min_feedback.max(1);
        let limit = query.limit.min(i64::MAX as usize);
        let offset = query.offset.min(i64::MAX as usize);
        let page = self.dislikes(XDislikeQuery {
            active: Some(true),
            limit: 500,
            offset: 0,
        })?;
        let mut groups = BTreeMap::<String, Vec<XDislikedPost>>::new();

        for item in page.items {
            let reason = clean_feedback_reason(&item.reason);
            groups.entry(reason).or_default().push(item);
        }

        let mut suggestions = groups
            .into_iter()
            .filter_map(|(reason, evidence)| {
                (evidence.len() >= min_feedback)
                    .then(|| rule_suggestion_from_feedback(reason, evidence))
            })
            .collect::<Vec<_>>();
        suggestions.sort_by(|left, right| {
            right
                .feedback_count
                .cmp(&left.feedback_count)
                .then_with(|| left.title.cmp(&right.title))
        });

        let total_matching = suggestions.len();
        let items = suggestions.into_iter().skip(offset).take(limit).collect();

        Ok(RuleSuggestionPage {
            total_matching,
            limit,
            offset,
            items,
        })
    }

    fn rule(&self, id: &str) -> super::Result<Option<ContentRule>> {
        Ok(self
            .connection
            .query_row(
                "
                SELECT
                    id,
                    site,
                    status,
                    priority,
                    title,
                    instruction,
                    created_source,
                    created_at_unix_ms,
                    updated_at_unix_ms,
                    examples_json
                FROM content_rules
                WHERE id = ?1
                ",
                [id],
                content_rule_from_row,
            )
            .optional()?)
    }

    fn rule_audit(&self, id: &str, limit: usize) -> super::Result<Vec<RuleAuditEvent>> {
        let limit = sqlite_limit(limit);
        let mut statement = self.connection.prepare(
            "
            SELECT
                id,
                rule_id,
                event_kind,
                source,
                created_at_unix_ms,
                snapshot_json
            FROM content_rule_events
            WHERE rule_id = ?1
            ORDER BY created_at_unix_ms DESC, id DESC
            LIMIT ?2
            ",
        )?;
        let events = statement
            .query_map(params![id, limit], rule_audit_event_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(events)
    }

    fn record_rule_event(
        &self,
        rule: &ContentRule,
        event_kind: &str,
        source: &str,
        created_at_unix_ms: i64,
    ) -> super::Result<()> {
        let snapshot_json = serde_json::to_string(rule)?;
        self.connection.execute(
            "
            INSERT INTO content_rule_events (
                rule_id,
                event_kind,
                source,
                created_at_unix_ms,
                snapshot_json
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ",
            params![
                rule.id.as_str(),
                event_kind,
                source,
                created_at_unix_ms,
                snapshot_json
            ],
        )?;

        Ok(())
    }

    pub(super) fn content_stats(&self) -> super::Result<ContentStats> {
        Ok(self.connection.query_row(
            "
            SELECT
                COUNT(*),
                COALESCE(SUM(seen_count), 0),
                COALESCE(SUM(CASE WHEN post_id IS NOT NULL THEN 1 ELSE 0 END), 0),
                MIN(first_seen_at_unix_ms),
                MAX(last_seen_at_unix_ms)
            FROM tweets
            ",
            [],
            |row| {
                let unique_items: i64 = row.get(0)?;
                let total_encounters: i64 = row.get(1)?;
                let items_with_stable_id: i64 = row.get(2)?;

                Ok(ContentStats {
                    content_kind: "post".into(),
                    unique_items: unique_items.max(0) as usize,
                    total_encounters: total_encounters.max(0) as usize,
                    items_with_stable_id: items_with_stable_id.max(0) as usize,
                    first_seen_at_unix_ms: row.get(3)?,
                    last_seen_at_unix_ms: row.get(4)?,
                })
            },
        )?)
    }

    pub(super) fn content(&self, query: ContentQuery) -> super::Result<ContentPage> {
        let limit = sqlite_limit(query.limit);
        let offset = sqlite_limit(query.offset);

        match clean_optional(query.search.as_deref()) {
            Some(search) => self.search_content(&search, limit, offset),
            None => self.list_content(limit, offset),
        }
    }

    pub(super) fn upsert_content_annotation(
        &mut self,
        input: ContentAnnotationInput,
    ) -> super::Result<ContentAnnotation> {
        let storage_key = input.storage_key.trim().to_string();
        let content_kind =
            clean_optional(Some(input.content_kind.as_str())).unwrap_or_else(|| "post".into());
        let annotation_type = input.annotation_type.trim().to_string();
        let annotation_key = input.key.trim().to_string();
        let source = input.source.trim().to_string();
        let value_json = serde_json::to_string(&input.value)?;
        let value_text = annotation_value_text(&input.value);
        let now = now_unix_ms();

        self.connection.execute(
            "
            INSERT INTO content_annotations (
                storage_key,
                content_kind,
                annotation_type,
                annotation_key,
                value_json,
                value_text,
                confidence,
                source,
                created_at_unix_ms,
                updated_at_unix_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
            ON CONFLICT(storage_key, annotation_type, annotation_key, source) DO UPDATE SET
                content_kind = excluded.content_kind,
                value_json = excluded.value_json,
                value_text = excluded.value_text,
                confidence = excluded.confidence,
                updated_at_unix_ms = excluded.updated_at_unix_ms
            ",
            params![
                storage_key,
                content_kind,
                annotation_type,
                annotation_key,
                value_json,
                value_text,
                input.confidence,
                source,
                now,
            ],
        )?;

        let annotation = self.connection.query_row(
            "
            SELECT
                id,
                storage_key,
                content_kind,
                annotation_type,
                annotation_key,
                value_json,
                confidence,
                source,
                created_at_unix_ms,
                updated_at_unix_ms
            FROM content_annotations
            WHERE storage_key = ?1
                AND annotation_type = ?2
                AND annotation_key = ?3
                AND source = ?4
            ",
            params![
                input.storage_key.trim(),
                input.annotation_type.trim(),
                input.key.trim(),
                input.source.trim(),
            ],
            content_annotation_from_row,
        )?;

        debug!(
            target: "weblayer_daemon::storage::x_com",
            storage_key = annotation.storage_key.as_str(),
            annotation_type = annotation.annotation_type.as_str(),
            key = annotation.key.as_str(),
            source = annotation.source.as_str(),
            "stored X content annotation"
        );

        Ok(annotation)
    }

    pub(super) fn content_annotations(
        &self,
        query: ContentAnnotationQuery,
    ) -> super::Result<ContentAnnotationPage> {
        let storage_key =
            annotation_storage_key(query.storage_key.as_deref(), query.content_id.as_deref());
        let content_kind = clean_optional(query.content_kind.as_deref());
        let annotation_type = clean_optional(query.annotation_type.as_deref());
        let annotation_key = clean_optional(query.key.as_deref());
        let source = clean_optional(query.source.as_deref());
        let limit = sqlite_limit(query.limit);
        let offset = sqlite_limit(query.offset);

        let total_matching = self.connection.query_row(
            "
            SELECT COUNT(*)
            FROM content_annotations
            WHERE (?1 IS NULL OR storage_key = ?1)
                AND (?2 IS NULL OR content_kind = ?2)
                AND (?3 IS NULL OR annotation_type = ?3)
                AND (?4 IS NULL OR annotation_key = ?4)
                AND (?5 IS NULL OR source = ?5)
            ",
            params![
                storage_key.as_deref(),
                content_kind.as_deref(),
                annotation_type.as_deref(),
                annotation_key.as_deref(),
                source.as_deref(),
            ],
            |row| row.get::<_, i64>(0),
        )?;

        let mut statement = self.connection.prepare(
            "
            SELECT
                id,
                storage_key,
                content_kind,
                annotation_type,
                annotation_key,
                value_json,
                confidence,
                source,
                created_at_unix_ms,
                updated_at_unix_ms
            FROM content_annotations
            WHERE (?1 IS NULL OR storage_key = ?1)
                AND (?2 IS NULL OR content_kind = ?2)
                AND (?3 IS NULL OR annotation_type = ?3)
                AND (?4 IS NULL OR annotation_key = ?4)
                AND (?5 IS NULL OR source = ?5)
            ORDER BY updated_at_unix_ms DESC, id DESC
            LIMIT ?6 OFFSET ?7
            ",
        )?;
        let items = statement
            .query_map(
                params![
                    storage_key.as_deref(),
                    content_kind.as_deref(),
                    annotation_type.as_deref(),
                    annotation_key.as_deref(),
                    source.as_deref(),
                    limit,
                    offset,
                ],
                content_annotation_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(ContentAnnotationPage {
            total_matching: total_matching.max(0) as usize,
            limit: limit as usize,
            offset: offset as usize,
            items,
        })
    }

    fn list_content(&self, limit: i64, offset: i64) -> super::Result<ContentPage> {
        let total_matching =
            self.connection
                .query_row("SELECT COUNT(*) FROM tweets", [], |row| {
                    row.get::<_, i64>(0)
                })?;

        let mut statement = self.connection.prepare(
            "
            SELECT
                storage_key,
                post_id,
                url,
                author_handle,
                text,
                first_seen_at_unix_ms,
                last_seen_at_unix_ms,
                seen_count,
                latest_captured_at
            FROM tweets
            ORDER BY last_seen_at_unix_ms DESC, storage_key ASC
            LIMIT ?1 OFFSET ?2
            ",
        )?;
        let items = statement
            .query_map(params![limit, offset], |row| {
                content_item_from_row(row, None)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(ContentPage {
            total_matching: total_matching.max(0) as usize,
            limit: limit as usize,
            offset: offset as usize,
            items,
        })
    }

    fn search_content(&self, search: &str, limit: i64, offset: i64) -> super::Result<ContentPage> {
        let Some(match_query) = fts_match_query(search) else {
            return Ok(ContentPage {
                total_matching: 0,
                limit: limit as usize,
                offset: offset as usize,
                items: Vec::new(),
            });
        };

        let total_matching = self.connection.query_row(
            "
            SELECT COUNT(*)
            FROM tweets_fts
            WHERE tweets_fts MATCH ?1
            ",
            [match_query.as_str()],
            |row| row.get::<_, i64>(0),
        )?;

        let mut statement = self.connection.prepare(
            "
            SELECT
                tweets.storage_key,
                tweets.post_id,
                tweets.url,
                tweets.author_handle,
                tweets.text,
                tweets.first_seen_at_unix_ms,
                tweets.last_seen_at_unix_ms,
                tweets.seen_count,
                tweets.latest_captured_at,
                snippet(tweets_fts, 0, '', '', '...', 24) AS snippet
            FROM tweets_fts
            JOIN tweets ON tweets.rowid = tweets_fts.rowid
            WHERE tweets_fts MATCH ?1
            ORDER BY bm25(tweets_fts), tweets.last_seen_at_unix_ms DESC, tweets.storage_key ASC
            LIMIT ?2 OFFSET ?3
            ",
        )?;
        let items = statement
            .query_map(params![match_query, limit, offset], |row| {
                content_item_from_row(row, row.get(9)?)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(ContentPage {
            total_matching: total_matching.max(0) as usize,
            limit: limit as usize,
            offset: offset as usize,
            items,
        })
    }
}

struct StoredTweet {
    storage_key: String,
    post_id: Option<String>,
    url: Option<String>,
    author_handle: Option<String>,
    text: String,
    normalized_text: String,
    text_hash: String,
    seen_at_unix_ms: i64,
    client_id: String,
    captured_at: Option<String>,
    payload_json: String,
}

struct StoredTweetFeedback {
    storage_key: String,
    post_id: Option<String>,
    feedback_kind: String,
    reason: String,
    created_at_unix_ms: i64,
    client_id: String,
    url: Option<String>,
    author_handle: Option<String>,
    captured_at: Option<String>,
    payload_json: String,
}

impl StoredTweet {
    fn from_item(
        source: &str,
        item: &ContentItem,
        seen_at_unix_ms: i64,
    ) -> super::Result<Option<Self>> {
        let post_id = stable_post_id(item);
        let normalized_text = normalize_text(&item.text);
        let storage_key = storage_key(item, post_id.as_deref(), &normalized_text);
        let Some(storage_key) = storage_key else {
            return Ok(None);
        };
        let payload_json = serde_json::to_string(&StoredTweetPayload {
            source,
            seen_at_unix_ms,
            item,
        })?;

        Ok(Some(Self {
            storage_key,
            post_id,
            url: clean_optional(item.url.as_deref()),
            author_handle: clean_optional(item.author.as_deref()),
            text: item.text.clone(),
            normalized_text: normalized_text.clone(),
            text_hash: format!("{:016x}", stable_hash(&normalized_text)),
            seen_at_unix_ms,
            client_id: item.client_id.clone(),
            captured_at: item.captured_at.clone(),
            payload_json,
        }))
    }
}

impl StoredTweetFeedback {
    fn from_item(
        item: &ContentItem,
        feedback: FeedbackKind,
        reason: &str,
        created_at_unix_ms: i64,
    ) -> super::Result<Option<Self>> {
        let post_id = stable_post_id(item);
        let normalized_text = normalize_text(&item.text);
        let Some(storage_key) = storage_key(item, post_id.as_deref(), &normalized_text) else {
            return Ok(None);
        };
        let feedback_kind = feedback_kind_name(feedback).to_string();
        let reason = reason.trim().to_string();
        let payload_json = serde_json::to_string(&StoredTweetFeedbackPayload {
            feedback_kind: feedback_kind.as_str(),
            reason: reason.as_str(),
            created_at_unix_ms,
            item,
        })?;

        Ok(Some(Self {
            storage_key,
            post_id,
            feedback_kind,
            reason,
            created_at_unix_ms,
            client_id: item.client_id.clone(),
            url: clean_optional(item.url.as_deref()),
            author_handle: clean_optional(item.author.as_deref()),
            captured_at: item.captured_at.clone(),
            payload_json,
        }))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredTweetPayload<'a> {
    source: &'a str,
    seen_at_unix_ms: i64,
    item: &'a ContentItem,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredTweetFeedbackPayload<'a> {
    feedback_kind: &'a str,
    reason: &'a str,
    created_at_unix_ms: i64,
    item: &'a ContentItem,
}

fn feedback_kind_name(feedback: FeedbackKind) -> &'static str {
    match feedback {
        FeedbackKind::ThumbsDown => "thumbsDown",
        FeedbackKind::UndoThumbsDown => "undoThumbsDown",
        FeedbackKind::UpdateReason => "updateReason",
    }
}

fn feedback_state_active(feedback: FeedbackKind) -> bool {
    match feedback {
        FeedbackKind::ThumbsDown | FeedbackKind::UpdateReason => true,
        FeedbackKind::UndoThumbsDown => false,
    }
}

fn bool_to_sqlite_int(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn sqlite_limit(value: usize) -> i64 {
    value.min(i64::MAX as usize) as i64
}

fn seed_default_rules(connection: &Connection) -> super::Result<()> {
    let now = now_unix_ms();
    let examples_json = serde_json::to_string(&RuleExamples {
        positive: Vec::new(),
        negative: Vec::new(),
    })?;

    let inserted = connection.execute(
        "
        INSERT OR IGNORE INTO content_rules (
            id,
            site,
            status,
            priority,
            title,
            instruction,
            created_source,
            created_at_unix_ms,
            updated_at_unix_ms,
            examples_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)
        ",
        params![
            DEFAULT_RULE_ID,
            SITE_DIR,
            DEFAULT_RULE_STATUS,
            DEFAULT_RULE_PRIORITY,
            DEFAULT_RULE_TITLE,
            DEFAULT_RULE_INSTRUCTION,
            DEFAULT_RULE_SOURCE,
            now,
            examples_json,
        ],
    )?;
    let event_count = connection.query_row(
        "
        SELECT COUNT(*)
        FROM content_rule_events
        WHERE rule_id = ?1
        ",
        [DEFAULT_RULE_ID],
        |row| row.get::<_, i64>(0),
    )?;
    if event_count == 0 {
        let rule = connection.query_row(
            "
            SELECT
                id,
                site,
                status,
                priority,
                title,
                instruction,
                created_source,
                created_at_unix_ms,
                updated_at_unix_ms,
                examples_json
            FROM content_rules
            WHERE id = ?1
            ",
            [DEFAULT_RULE_ID],
            content_rule_from_row,
        )?;
        let snapshot_json = serde_json::to_string(&rule)?;
        let event_kind = if inserted > 0 { "created" } else { "imported" };
        connection.execute(
            "
            INSERT INTO content_rule_events (
                rule_id,
                event_kind,
                source,
                created_at_unix_ms,
                snapshot_json
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ",
            params![
                DEFAULT_RULE_ID,
                event_kind,
                rule.created_source,
                now,
                snapshot_json
            ],
        )?;
    }

    Ok(())
}

fn migrate_search(connection: &Connection) -> super::Result<()> {
    connection.execute_batch(
        "
        CREATE VIRTUAL TABLE IF NOT EXISTS tweets_fts USING fts5(
            text,
            author_handle,
            url,
            content='tweets',
            content_rowid='rowid',
            tokenize='unicode61'
        );

        CREATE TRIGGER IF NOT EXISTS tweets_fts_after_insert
        AFTER INSERT ON tweets BEGIN
            INSERT INTO tweets_fts(rowid, text, author_handle, url)
            VALUES (new.rowid, new.text, new.author_handle, new.url);
        END;

        CREATE TRIGGER IF NOT EXISTS tweets_fts_after_delete
        AFTER DELETE ON tweets BEGIN
            INSERT INTO tweets_fts(tweets_fts, rowid, text, author_handle, url)
            VALUES ('delete', old.rowid, old.text, old.author_handle, old.url);
        END;

        CREATE TRIGGER IF NOT EXISTS tweets_fts_after_update
        AFTER UPDATE ON tweets BEGIN
            INSERT INTO tweets_fts(tweets_fts, rowid, text, author_handle, url)
            VALUES ('delete', old.rowid, old.text, old.author_handle, old.url);
            INSERT INTO tweets_fts(rowid, text, author_handle, url)
            VALUES (new.rowid, new.text, new.author_handle, new.url);
        END;

        INSERT INTO tweets_fts(tweets_fts) VALUES ('rebuild');
        ",
    )?;

    Ok(())
}

fn content_item_from_row(
    row: &Row<'_>,
    snippet: Option<String>,
) -> rusqlite::Result<StoredContentItem> {
    Ok(StoredContentItem {
        content_kind: "post".into(),
        storage_key: row.get(0)?,
        content_id: row.get(1)?,
        url: row.get(2)?,
        author: row.get(3)?,
        text: row.get(4)?,
        snippet,
        first_seen_at_unix_ms: row.get(5)?,
        last_seen_at_unix_ms: row.get(6)?,
        seen_count: row.get(7)?,
        latest_captured_at: row.get(8)?,
    })
}

fn content_annotation_from_row(row: &Row<'_>) -> rusqlite::Result<ContentAnnotation> {
    let value_json: String = row.get(5)?;
    let value = serde_json::from_str(&value_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;

    Ok(ContentAnnotation {
        id: row.get(0)?,
        storage_key: row.get(1)?,
        content_kind: row.get(2)?,
        annotation_type: row.get(3)?,
        key: row.get(4)?,
        value,
        confidence: row.get(6)?,
        source: row.get(7)?,
        created_at_unix_ms: row.get(8)?,
        updated_at_unix_ms: row.get(9)?,
    })
}

fn content_rule_from_row(row: &Row<'_>) -> rusqlite::Result<ContentRule> {
    let examples_json: String = row.get(9)?;
    let examples = serde_json::from_str(&examples_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(error))
    })?;

    Ok(ContentRule {
        id: row.get(0)?,
        site: row.get(1)?,
        status: row.get(2)?,
        priority: row.get(3)?,
        title: row.get(4)?,
        instruction: row.get(5)?,
        created_source: row.get(6)?,
        created_at_unix_ms: row.get(7)?,
        updated_at_unix_ms: row.get(8)?,
        examples,
    })
}

fn rule_audit_event_from_row(row: &Row<'_>) -> rusqlite::Result<RuleAuditEvent> {
    let snapshot_json: String = row.get(5)?;
    let snapshot = serde_json::from_str(&snapshot_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;

    Ok(RuleAuditEvent {
        id: row.get(0)?,
        rule_id: row.get(1)?,
        event_kind: row.get(2)?,
        source: row.get(3)?,
        created_at_unix_ms: row.get(4)?,
        snapshot,
    })
}

fn annotation_storage_key(storage_key: Option<&str>, content_id: Option<&str>) -> Option<String> {
    clean_optional(storage_key)
        .or_else(|| clean_optional(content_id).map(|content_id| format!("x:id:{content_id}")))
}

fn annotation_value_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn fts_match_query(search: &str) -> Option<String> {
    let tokens = search
        .split(|character: char| !character.is_alphanumeric())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect::<Vec<_>>();

    (!tokens.is_empty()).then(|| tokens.join(" AND "))
}

fn storage_key(item: &ContentItem, post_id: Option<&str>, normalized_text: &str) -> Option<String> {
    if let Some(post_id) = post_id {
        return Some(format!("x:id:{post_id}"));
    }

    let author = item.author.as_deref().unwrap_or_default().trim();
    let url = item.url.as_deref().map(normalize_url).unwrap_or_default();

    if author.is_empty() && url.is_empty() && normalized_text.is_empty() {
        return None;
    }

    Some(format!(
        "x:fallback:{:016x}",
        stable_hash(&format!(
            "author={}\nurl={}\ntext={}",
            author, url, normalized_text
        ))
    ))
}

fn stable_post_id(item: &ContentItem) -> Option<String> {
    item.content_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            item.url
                .as_deref()
                .and_then(x_status_id)
                .map(ToOwned::to_owned)
        })
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

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn required_clean_text(value: &str, field: &str) -> super::Result<String> {
    clean_optional(Some(value))
        .ok_or_else(|| super::StorageError::InvalidInput(format!("{field} must not be empty")))
}

fn clean_rule_id(value: &str) -> super::Result<String> {
    let id = required_clean_text(value, "id")?;
    let valid = id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_');
    if !valid {
        return Err(super::StorageError::InvalidInput(
            "id may contain only ASCII letters, numbers, hyphens, and underscores".into(),
        ));
    }

    Ok(id)
}

fn clean_rule_id_for_lookup(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn clean_rule_status(value: &str) -> super::Result<String> {
    let status = required_clean_text(value, "status")?.to_ascii_lowercase();
    match status.as_str() {
        "draft" | "active" | "disabled" | "archived" => Ok(status),
        _ => Err(super::StorageError::InvalidInput(format!(
            "unsupported rule status: {status}"
        ))),
    }
}

fn clean_examples(examples: RuleExamples) -> RuleExamples {
    RuleExamples {
        positive: clean_example_list(examples.positive),
        negative: clean_example_list(examples.negative),
    }
}

fn clean_example_list(examples: Vec<String>) -> Vec<String> {
    let mut cleaned = Vec::new();
    for example in examples {
        let example = normalize_text(&example);
        if example.is_empty() || cleaned.contains(&example) {
            continue;
        }
        cleaned.push(example);
    }

    cleaned
}

fn generated_rule_id(title: &str, instruction: &str, now_unix_ms: i64) -> String {
    let slug = rule_slug(title);
    let hash = stable_hash(&format!("{title}\n{instruction}\n{now_unix_ms}"));
    format!("x-{slug}-{:08x}", hash as u32)
}

fn rule_slug(title: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;

    for character in title.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_was_separator = false;
        } else if !last_was_separator && !slug.is_empty() {
            slug.push('-');
            last_was_separator = true;
        }

        if slug.len() >= 48 {
            break;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "rule".into()
    } else {
        slug
    }
}

fn rule_status_event_kind(status: &str) -> &'static str {
    match status {
        "active" => "enabled",
        "disabled" => "disabled",
        "archived" => "archived",
        "draft" => "drafted",
        _ => "statusChanged",
    }
}

struct RuleMatcher {
    terms: Vec<String>,
    positive_examples: Vec<String>,
    negative_examples: Vec<String>,
}

impl RuleMatcher {
    fn new(rule: &ContentRule) -> Self {
        let mut terms = Vec::new();
        for source in [&rule.title, &rule.instruction] {
            collect_match_terms(source, &mut terms);
        }
        for example in &rule.examples.positive {
            collect_match_terms(example, &mut terms);
        }

        Self {
            terms,
            positive_examples: normalized_examples(&rule.examples.positive),
            negative_examples: normalized_examples(&rule.examples.negative),
        }
    }

    fn evaluate(&self, content: StoredContentItem) -> Option<RuleValidationMatch> {
        let text = content.text.to_ascii_lowercase();
        if text.is_empty()
            || self
                .negative_examples
                .iter()
                .any(|example| text.contains(example))
        {
            return None;
        }

        let matched_terms = self
            .terms
            .iter()
            .filter(|term| text_contains_token(&text, term))
            .cloned()
            .collect::<Vec<_>>();
        let matched_examples = self
            .positive_examples
            .iter()
            .filter(|example| text.contains(*example))
            .cloned()
            .collect::<Vec<_>>();

        if matched_terms.is_empty() && matched_examples.is_empty() {
            return None;
        }

        let score = matched_terms.len() + matched_examples.len() * 5;
        Some(RuleValidationMatch {
            content,
            score,
            matched_terms,
            matched_examples,
        })
    }
}

fn collect_match_terms(text: &str, terms: &mut Vec<String>) {
    for token in text
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 4)
        .map(str::to_ascii_lowercase)
    {
        if is_rule_match_stop_word(&token) || terms.contains(&token) {
            continue;
        }
        terms.push(token);
    }
}

fn normalized_examples(examples: &[String]) -> Vec<String> {
    examples
        .iter()
        .map(|example| normalize_text(example).to_ascii_lowercase())
        .filter(|example| example.len() >= 8)
        .collect()
}

fn text_contains_token(text: &str, token: &str) -> bool {
    text.split(|character: char| !character.is_ascii_alphanumeric())
        .any(|candidate| candidate.eq_ignore_ascii_case(token))
}

fn is_rule_match_stop_word(token: &str) -> bool {
    matches!(
        token,
        "about"
            | "active"
            | "after"
            | "again"
            | "against"
            | "because"
            | "content"
            | "downrank"
            | "from"
            | "hide"
            | "like"
            | "main"
            | "more"
            | "post"
            | "posts"
            | "rather"
            | "rule"
            | "rules"
            | "should"
            | "site"
            | "source"
            | "status"
            | "than"
            | "that"
            | "this"
            | "tweet"
            | "tweets"
            | "where"
            | "with"
    )
}

fn clean_feedback_reason(reason: &str) -> String {
    let reason = normalize_text(reason);
    if reason.is_empty() {
        "thumbs-down feedback".into()
    } else {
        reason
    }
}

fn rule_suggestion_from_feedback(
    reason: String,
    mut evidence: Vec<XDislikedPost>,
) -> RuleSuggestion {
    evidence.sort_by(|left, right| {
        right
            .updated_at_unix_ms
            .cmp(&left.updated_at_unix_ms)
            .then_with(|| left.storage_key.cmp(&right.storage_key))
    });
    let title_reason = sentence_case(&reason);
    let title = format!("Feedback: {}", truncate_for_rule(&title_reason, 72));
    let instruction = format!(
        "Hide X posts similar to posts the user disliked for this reason: {}.",
        reason
    );
    let examples = RuleExamples {
        positive: evidence
            .iter()
            .filter_map(|item| clean_optional(Some(item.text.as_str())))
            .take(5)
            .collect(),
        negative: Vec::new(),
    };
    let id = format!(
        "x-feedback-{}-{:08x}",
        rule_slug(&reason),
        stable_hash(&reason) as u32
    );

    RuleSuggestion {
        id,
        status: DEFAULT_NEW_RULE_STATUS.into(),
        priority: DEFAULT_NEW_RULE_PRIORITY,
        title,
        instruction,
        source: "feedback".into(),
        feedback_count: evidence.len(),
        reasons: vec![reason],
        examples,
        evidence: evidence.into_iter().take(10).collect(),
    }
}

fn sentence_case(text: &str) -> String {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };

    format!("{}{}", first.to_uppercase(), chars.collect::<String>())
}

fn truncate_for_rule(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_url(url: &str) -> String {
    let trimmed = url.trim();
    let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);
    without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment)
        .to_string()
}

fn stable_hash(text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;

    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }

    hash
}

fn now_unix_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    millis.min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn records_thumbs_down_feedback_for_x_post() {
        let db_path = temp_db_path("records-feedback");
        let mut store = Store::open(&db_path).expect("store should open");
        let item = item("client-1", Some("123"), "hello");

        let recorded = store
            .record_feedback(&item, FeedbackKind::ThumbsDown, "")
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
            .record_feedback(&item, FeedbackKind::ThumbsDown, "")
            .expect("feedback should store");
        store
            .record_feedback(&item, FeedbackKind::UpdateReason, "low information")
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
            .record_feedback(&item, FeedbackKind::ThumbsDown, "low information")
            .expect("feedback should store");
        store
            .record_feedback(&item, FeedbackKind::UndoThumbsDown, "")
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
            .record_feedback(&active_item, FeedbackKind::ThumbsDown, "low information")
            .expect("active feedback should store");
        store
            .record_feedback(&inactive_item, FeedbackKind::ThumbsDown, "spam")
            .expect("inactive feedback should store");
        store
            .record_feedback(&inactive_item, FeedbackKind::UndoThumbsDown, "")
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
            .record_feedback(&first, FeedbackKind::ThumbsDown, "engagement bait")
            .expect("feedback should store");
        store
            .record_feedback(&second, FeedbackKind::ThumbsDown, "engagement bait")
            .expect("feedback should store");
        store
            .record_feedback(&ignored, FeedbackKind::ThumbsDown, "spam")
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
}
