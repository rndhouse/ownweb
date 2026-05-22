use crate::core::{AnalysisBatch, ContentItem};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{debug, info};

const DATA_DIR_ENV: &str = "OWNWEB_DATA_DIR";
const X_SITE_DIR: &str = "x.com";
const DB_FILE_NAME: &str = "db.sqlite";

/// Filesystem-backed storage for content encountered by the daemon.
#[derive(Clone)]
pub struct ContentStore {
    x_com: Arc<Mutex<WebsiteDb>>,
}

impl ContentStore {
    /// Opens the per-site storage databases under the configured data directory.
    pub fn from_env() -> Result<Self> {
        Self::with_data_dir(data_dir_from_env()?)
    }

    fn with_data_dir(data_dir: impl AsRef<Path>) -> Result<Self> {
        let x_path = site_db_path(data_dir.as_ref(), X_SITE_DIR);
        let x_com = WebsiteDb::open_x_com(&x_path)?;

        info!(
            site = X_SITE_DIR,
            path = %x_path.display(),
            "opened site storage database"
        );

        Ok(Self {
            x_com: Arc::new(Mutex::new(x_com)),
        })
    }

    /// Stores content for supported sites before analysis.
    pub fn record_batch(&self, batch: &AnalysisBatch) -> Result<()> {
        if !is_x_source(&batch.source) {
            return Ok(());
        }

        let mut db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.record_x_batch(batch)
    }
}

struct WebsiteDb {
    connection: Connection,
}

impl WebsiteDb {
    fn open_x_com(path: &Path) -> Result<Self> {
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

    fn record_x_batch(&mut self, batch: &AnalysisBatch) -> Result<()> {
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
            target: "ownweb_daemon::storage",
            source = batch.source.as_str(),
            stored_count,
            skipped_count,
            "stored content batch"
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

fn is_x_source(source: &str) -> bool {
    matches!(
        source.trim().to_ascii_lowercase().as_str(),
        "x.com" | "www.x.com" | "twitter.com" | "www.twitter.com"
    )
}

fn site_db_path(data_dir: &Path, site: &str) -> PathBuf {
    data_dir.join(site).join(DB_FILE_NAME)
}

fn data_dir_from_env() -> Result<PathBuf> {
    if let Some(path) = non_empty_env(DATA_DIR_ENV) {
        return Ok(PathBuf::from(path));
    }

    if let Some(home) = non_empty_env("HOME") {
        return Ok(PathBuf::from(home).join(".local/share/ownweb"));
    }

    Err(StorageError::MissingDataDir)
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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

type Result<T> = std::result::Result<T, StorageError>;

/// Error returned by the filesystem-backed content store.
#[derive(Debug)]
pub enum StorageError {
    /// No data directory could be determined from `OWNWEB_DATA_DIR` or `HOME`.
    MissingDataDir,
    /// Filesystem setup failed.
    Io(std::io::Error),
    /// SQLite failed to open, migrate, or write.
    Sqlite(rusqlite::Error),
    /// Payload JSON serialization failed.
    Json(serde_json::Error),
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDataDir => write!(
                formatter,
                "missing data directory; set OWNWEB_DATA_DIR or HOME"
            ),
            Self::Io(error) => write!(formatter, "filesystem error: {error}"),
            Self::Sqlite(error) => write!(formatter, "sqlite error: {error}"),
            Self::Json(error) => write!(formatter, "json error: {error}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

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

    fn temp_data_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("ownweb-storage-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn stores_x_posts_in_site_database() {
        let data_dir = temp_data_dir("stores-x-dom-posts");
        let store = ContentStore::with_data_dir(&data_dir).expect("store should open");

        store
            .record_batch(&batch(
                "x.com",
                vec![item("client-1", Some("123"), "hello")],
            ))
            .expect("batch should store");

        let db_path = data_dir.join("x.com/db.sqlite");
        assert!(db_path.exists());

        let connection = Connection::open(&db_path).expect("database should open");
        let text: String = connection
            .query_row(
                "SELECT text FROM tweets WHERE storage_key = 'x:id:123'",
                [],
                |row| row.get(0),
            )
            .expect("tweet should exist");
        assert_eq!(text, "hello");

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn upserts_x_posts_by_post_id() {
        let data_dir = temp_data_dir("upserts-x-dom-posts");
        let store = ContentStore::with_data_dir(&data_dir).expect("store should open");

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

        let connection =
            Connection::open(data_dir.join("x.com/db.sqlite")).expect("database should open");
        let (text, seen_count, latest_client_id): (String, i64, String) = connection
            .query_row(
                "SELECT text, seen_count, latest_client_id FROM tweets WHERE storage_key = 'x:id:123'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("tweet should exist");

        assert_eq!(text, "second");
        assert_eq!(seen_count, 2);
        assert_eq!(latest_client_id, "client-2");

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn non_x_sources_do_not_create_site_databases() {
        let data_dir = temp_data_dir("non-x-sources");
        let store = ContentStore::with_data_dir(&data_dir).expect("store should open");

        store
            .record_batch(&batch(
                "example.com",
                vec![item("client-1", Some("123"), "hello")],
            ))
            .expect("unsupported source should be ignored");

        let count: i64 = store
            .x_com
            .lock()
            .expect("mutex should lock")
            .connection
            .query_row("SELECT COUNT(*) FROM tweets", [], |row| row.get(0))
            .expect("count should be readable");

        assert_eq!(count, 0);

        let _ = std::fs::remove_dir_all(data_dir);
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
