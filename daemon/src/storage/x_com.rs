use crate::core::{AnalysisBatch, ContentItem};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::debug;

pub(super) const SITE_DIR: &str = "x.com";

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
            ",
        )?;

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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredTweetPayload<'a> {
    source: &'a str,
    seen_at_unix_ms: i64,
    item: &'a ContentItem,
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
    use serde_json::Value;
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
