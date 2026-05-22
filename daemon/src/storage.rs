mod x_com;

use crate::core::AnalysisBatch;
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tracing::info;

const DATA_DIR_ENV: &str = "OWNWEB_DATA_DIR";
const DB_FILE_NAME: &str = "db.sqlite";

/// Filesystem-backed storage for content encountered by the daemon.
#[derive(Clone)]
pub struct ContentStore {
    x_com: Arc<Mutex<x_com::Store>>,
}

impl ContentStore {
    /// Opens the per-site storage databases under the configured data directory.
    pub fn from_env() -> Result<Self> {
        Self::with_data_dir(data_dir_from_env()?)
    }

    fn with_data_dir(data_dir: impl AsRef<Path>) -> Result<Self> {
        let x_com_path = site_db_path(data_dir.as_ref(), x_com::SITE_DIR);
        let x_com = x_com::Store::open(&x_com_path)?;
        log_site_db_opened(x_com::SITE_DIR, &x_com_path);

        Ok(Self {
            x_com: Arc::new(Mutex::new(x_com)),
        })
    }

    /// Stores X/Twitter content in the X site database.
    pub fn record_x_batch(&self, batch: &AnalysisBatch) -> Result<()> {
        let mut db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.record_batch(batch)
    }
}

fn log_site_db_opened(site: &str, path: &Path) {
    info!(
        site,
        path = %path.display(),
        "opened site storage database"
    );
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
    use crate::core::ContentItem;
    use rusqlite::Connection;
    use serde_json::Value;

    fn item(client_id: &str, post_id: &str, text: &str) -> ContentItem {
        ContentItem {
            client_id: client_id.into(),
            content_id: Some(post_id.into()),
            url: Some(format!("https://x.com/user/status/{post_id}?utm=1")),
            author: Some("@user".into()),
            text: text.into(),
            captured_at: Some("2026-05-21T12:00:00.000Z".into()),
            kind: Some("post".into()),
            metadata: Value::Null,
        }
    }

    fn temp_data_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("ownweb-storage-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn content_store_records_x_batches_in_x_site_database() {
        let data_dir = temp_data_dir("content-store-records-x");
        let store = ContentStore::with_data_dir(&data_dir).expect("store should open");

        store
            .record_x_batch(&AnalysisBatch::new(
                "x.com",
                vec![item("client-1", "123", "hello")],
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
}
