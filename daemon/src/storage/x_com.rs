use crate::{
    core::{AnalysisBatch, ContentItem, FeedbackKind},
    storage::{
        ContentAnnotation, ContentAnnotationInput, ContentAnnotationPage, ContentAnnotationQuery,
        ContentPage, ContentQuery, ContentRule, ContentStats, RuleExamples, RulePage, RuleQuery,
        StoredContentItem, XDislikePage, XDislikeQuery, XDislikedPost,
    },
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::{
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
const DEFAULT_RULE_SOURCE: &str = "user";

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
            target: "ownweb_daemon::storage::x_com",
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
                target: "ownweb_daemon::storage::x_com",
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
            target: "ownweb_daemon::storage::x_com",
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
            target: "ownweb_daemon::storage::x_com",
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

    connection.execute(
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
            "ownweb-x-storage-test-{name}-{}",
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
