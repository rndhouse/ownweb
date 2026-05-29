use super::super::Result;
use super::{
    clean_optional, normalize_text, now_unix_ms, sqlite_limit, stable_hash, stable_post_id,
    storage_key, Store,
};
use crate::{
    core::{AnalysisBatch, ContentItem},
    storage::{ContentPage, ContentQuery, ContentStats, StoredContentItem},
};
use rusqlite::{params, Row};
use serde::Serialize;
use tracing::debug;

impl Store {
    pub(in crate::storage) fn record_batch(&mut self, batch: &AnalysisBatch) -> Result<()> {
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

    pub(in crate::storage) fn content_stats(&self) -> Result<ContentStats> {
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

    pub(in crate::storage) fn content(&self, query: ContentQuery) -> Result<ContentPage> {
        let limit = sqlite_limit(query.limit);
        let offset = sqlite_limit(query.offset);

        match clean_optional(query.search.as_deref()) {
            Some(search) => self.search_content(&search, limit, offset),
            None => self.list_content(limit, offset),
        }
    }

    fn list_content(&self, limit: i64, offset: i64) -> Result<ContentPage> {
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

    fn search_content(&self, search: &str, limit: i64, offset: i64) -> Result<ContentPage> {
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

impl StoredTweet {
    fn from_item(source: &str, item: &ContentItem, seen_at_unix_ms: i64) -> Result<Option<Self>> {
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredTweetPayload<'a> {
    source: &'a str,
    seen_at_unix_ms: i64,
    item: &'a ContentItem,
}

pub(super) fn content_item_from_row(
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

fn fts_match_query(search: &str) -> Option<String> {
    let tokens = search
        .split(|character: char| !character.is_alphanumeric())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect::<Vec<_>>();

    (!tokens.is_empty()).then(|| tokens.join(" AND "))
}
