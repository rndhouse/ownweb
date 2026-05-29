use super::super::{Result, XFeedbackState};
use super::{
    clean_optional, normalize_text, now_unix_ms, sqlite_limit, stable_post_id, storage_key, Store,
};
use crate::{
    core::{ContentItem, FeedbackContext, FeedbackKind},
    storage::{XDislikePage, XDislikeQuery, XDislikedPost},
};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use tracing::debug;

impl Store {
    pub(in crate::storage) fn record_feedback_with_context(
        &mut self,
        item: &ContentItem,
        feedback: FeedbackKind,
        reason: &str,
        feedback_context: &FeedbackContext,
    ) -> Result<bool> {
        let created_at_unix_ms = now_unix_ms();
        let Some(record) = StoredTweetFeedback::from_item(
            item,
            feedback,
            reason,
            created_at_unix_ms,
            feedback_context,
        )?
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
                payload_json,
                rule_context_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
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
                record.rule_context_json,
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
                latest_payload_json,
                latest_rule_context_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
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
                latest_payload_json = excluded.latest_payload_json,
                latest_rule_context_json = excluded.latest_rule_context_json
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
                record.rule_context_json,
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

    pub(in crate::storage) fn feedback_state(
        &self,
        item: &ContentItem,
    ) -> Result<Option<XFeedbackState>> {
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
                    Ok(XFeedbackState {
                        active: active != 0,
                        reason,
                    })
                },
            )
            .optional()?;

        Ok(state)
    }

    pub(in crate::storage) fn dislikes(&self, query: XDislikeQuery) -> Result<XDislikePage> {
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
                COALESCE(state.latest_captured_at, tweets.latest_captured_at) AS latest_captured_at,
                state.latest_rule_context_json
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
                    rule_context: feedback_context_from_row(row, 14)?,
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
    rule_context_json: String,
}

impl StoredTweetFeedback {
    fn from_item(
        item: &ContentItem,
        feedback: FeedbackKind,
        reason: &str,
        created_at_unix_ms: i64,
        feedback_context: &FeedbackContext,
    ) -> Result<Option<Self>> {
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
            rule_context: feedback_context,
        })?;
        let rule_context_json = serde_json::to_string(feedback_context)?;

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
            rule_context_json,
        }))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredTweetFeedbackPayload<'a> {
    feedback_kind: &'a str,
    reason: &'a str,
    created_at_unix_ms: i64,
    item: &'a ContentItem,
    rule_context: &'a FeedbackContext,
}

fn feedback_context_from_row(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<FeedbackContext> {
    let json: String = row.get(index)?;
    serde_json::from_str(&json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
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
