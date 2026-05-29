mod x_com;

use crate::core::{AnalysisBatch, ContentItem, FeedbackContext, FeedbackKind};
use serde::Serialize;
use serde_json::Value;
use std::{
    fmt,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tracing::{info, warn};

const DATA_DIR_ENV: &str = "WEBLAYER_DATA_DIR";
const RESET_X_DB_ENV: &str = "WEBLAYER_X_RESET_DB";
const DB_FILE_NAME: &str = "db.sqlite";
const DEFAULT_RULE_LIMIT: usize = 100;

/// Filesystem-backed storage for content encountered by the daemon.
#[derive(Clone)]
pub struct ContentStore {
    x_com: Arc<Mutex<x_com::Store>>,
}

/// Current feedback state for one stored X/Twitter content item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XFeedbackState {
    /// Whether this item is currently disliked by the user.
    pub active: bool,
    /// Latest reason attached to the dislike signal.
    pub reason: String,
}

/// Query options for listing stored X/Twitter dislikes.
#[derive(Debug, Clone, Copy)]
pub struct XDislikeQuery {
    /// Whether to filter by current active dislike state.
    pub active: Option<bool>,
    /// Maximum number of rows to return.
    pub limit: usize,
    /// Number of matching rows to skip.
    pub offset: usize,
}

/// Page of stored X/Twitter dislikes.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct XDislikePage {
    /// Number of matching rows before pagination.
    pub total_matching: usize,
    /// Maximum number of rows requested.
    pub limit: usize,
    /// Number of matching rows skipped.
    pub offset: usize,
    /// Matching disliked posts.
    pub items: Vec<XDislikedPost>,
}

/// Stored X/Twitter post plus user feedback state.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct XDislikedPost {
    /// Stable storage key used by WebLayer.
    pub storage_key: String,
    /// X status ID when known.
    pub post_id: Option<String>,
    /// Canonical or latest captured URL when known.
    pub url: Option<String>,
    /// Author handle when known.
    pub author: Option<String>,
    /// Latest stored post text.
    pub text: String,
    /// Latest user-entered feedback reason.
    pub reason: String,
    /// Whether this dislike is currently active.
    pub active: bool,
    /// Latest feedback event kind that set this state.
    pub feedback_kind: String,
    /// First timestamp for this feedback state.
    pub disliked_at_unix_ms: i64,
    /// Latest timestamp for this feedback state.
    pub updated_at_unix_ms: i64,
    /// First time this post was seen.
    pub first_seen_at_unix_ms: Option<i64>,
    /// Most recent time this post was seen.
    pub last_seen_at_unix_ms: Option<i64>,
    /// Number of times this post has been stored.
    pub seen_count: Option<i64>,
    /// Latest client-side capture timestamp.
    pub latest_captured_at: Option<String>,
    /// Rule context captured with the latest feedback event.
    pub rule_context: FeedbackContext,
}

/// Aggregate counts for content stored under one site scope.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContentStats {
    /// Logical content kind for these counts.
    pub content_kind: String,
    /// Number of unique stored content rows.
    pub unique_items: usize,
    /// Total number of captured encounters across stored rows.
    pub total_encounters: usize,
    /// Number of rows with a stable source content ID.
    pub items_with_stable_id: usize,
    /// Earliest time any row in this scope was first seen.
    pub first_seen_at_unix_ms: Option<i64>,
    /// Latest time any row in this scope was seen.
    pub last_seen_at_unix_ms: Option<i64>,
}

/// Query options for listing or searching stored content.
#[derive(Debug, Clone)]
pub struct ContentQuery {
    /// Optional full-text search query.
    pub search: Option<String>,
    /// Maximum number of rows to return.
    pub limit: usize,
    /// Number of matching rows to skip.
    pub offset: usize,
}

/// Page of stored content rows.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContentPage {
    /// Number of matching rows before pagination.
    pub total_matching: usize,
    /// Maximum number of rows requested.
    pub limit: usize,
    /// Number of matching rows skipped.
    pub offset: usize,
    /// Matching stored content rows.
    pub items: Vec<StoredContentItem>,
}

/// Stored content row returned by inspection endpoints.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredContentItem {
    /// Logical content kind, such as `post`.
    pub content_kind: String,
    /// Stable storage key used by WebLayer.
    pub storage_key: String,
    /// Stable source content ID when known.
    pub content_id: Option<String>,
    /// Canonical or latest captured URL when known.
    pub url: Option<String>,
    /// Author or account handle when known.
    pub author: Option<String>,
    /// Latest stored text.
    pub text: String,
    /// Search result excerpt when a search query was used.
    pub snippet: Option<String>,
    /// First time this item was seen.
    pub first_seen_at_unix_ms: i64,
    /// Most recent time this item was seen.
    pub last_seen_at_unix_ms: i64,
    /// Number of times this item has been stored.
    pub seen_count: i64,
    /// Latest client-side capture timestamp.
    pub latest_captured_at: Option<String>,
}

/// Agent- or user-supplied metadata to attach to one stored content item.
#[derive(Debug, Clone)]
pub struct ContentAnnotationInput {
    /// Stable storage key returned by content inspection endpoints.
    pub storage_key: String,
    /// Logical content kind, such as `post`.
    pub content_kind: String,
    /// Annotation category, such as `tag`, `note`, or `topic`.
    pub annotation_type: String,
    /// Annotation key within its category.
    pub key: String,
    /// Arbitrary annotation payload.
    pub value: Value,
    /// Optional model confidence from 0.0 to 1.0.
    pub confidence: Option<f64>,
    /// Source that created or updated this annotation.
    pub source: String,
}

/// Query options for listing stored content annotations.
#[derive(Debug, Clone)]
pub struct ContentAnnotationQuery {
    /// Optional stable storage key filter.
    pub storage_key: Option<String>,
    /// Optional site-native content ID filter.
    pub content_id: Option<String>,
    /// Optional logical content kind filter.
    pub content_kind: Option<String>,
    /// Optional annotation category filter.
    pub annotation_type: Option<String>,
    /// Optional annotation key filter.
    pub key: Option<String>,
    /// Optional source filter.
    pub source: Option<String>,
    /// Maximum number of rows to return.
    pub limit: usize,
    /// Number of matching rows to skip.
    pub offset: usize,
}

/// Page of stored content annotations.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContentAnnotationPage {
    /// Number of matching rows before pagination.
    pub total_matching: usize,
    /// Maximum number of rows requested.
    pub limit: usize,
    /// Number of matching rows skipped.
    pub offset: usize,
    /// Matching annotations.
    pub items: Vec<ContentAnnotation>,
}

/// Stored content annotation returned by inspection endpoints.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContentAnnotation {
    /// Database row ID for this annotation.
    pub id: i64,
    /// Stable storage key used by WebLayer.
    pub storage_key: String,
    /// Logical content kind, such as `post`.
    pub content_kind: String,
    /// Annotation category, such as `tag`, `note`, or `topic`.
    pub annotation_type: String,
    /// Annotation key within its category.
    pub key: String,
    /// Arbitrary annotation payload.
    pub value: Value,
    /// Optional model confidence from 0.0 to 1.0.
    pub confidence: Option<f64>,
    /// Source that created or updated this annotation.
    pub source: String,
    /// Annotation creation timestamp.
    pub created_at_unix_ms: i64,
    /// Annotation update timestamp.
    pub updated_at_unix_ms: i64,
}

/// Query options for listing content rules.
#[derive(Debug, Clone)]
pub struct RuleQuery {
    /// Optional site-specific status filter such as `active` or `draft`.
    pub status: Option<String>,
    /// Maximum number of rows to return.
    pub limit: usize,
    /// Number of matching rows to skip.
    pub offset: usize,
}

impl Default for RuleQuery {
    fn default() -> Self {
        Self {
            status: None,
            limit: DEFAULT_RULE_LIMIT,
            offset: 0,
        }
    }
}

/// Page of stored content rules.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RulePage {
    /// Number of matching rows before pagination.
    pub total_matching: usize,
    /// Maximum number of rows requested.
    pub limit: usize,
    /// Number of matching rows skipped.
    pub offset: usize,
    /// Matching rules.
    pub items: Vec<ContentRule>,
}

/// Stored user policy rule available to filtering agents.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContentRule {
    /// Stable rule ID.
    pub id: String,
    /// Site scope for the rule.
    pub site: String,
    /// Rule lifecycle status such as `draft`, `active`, or `disabled`.
    pub status: String,
    /// Lower numbers run earlier when multiple rules are active.
    pub priority: i64,
    /// Short human-readable rule name.
    pub title: String,
    /// Agent-facing instruction text.
    pub instruction: String,
    /// Source that created this rule.
    pub created_source: String,
    /// Rule creation timestamp.
    pub created_at_unix_ms: i64,
    /// Rule update timestamp.
    pub updated_at_unix_ms: i64,
    /// Examples attached to this rule.
    pub examples: RuleExamples,
}

/// Positive and negative examples for one content rule.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuleExamples {
    /// Examples that should match this rule.
    #[serde(default)]
    pub positive: Vec<String>,
    /// Examples that should not match this rule.
    #[serde(default)]
    pub negative: Vec<String>,
}

impl Default for RuleExamples {
    fn default() -> Self {
        Self {
            positive: Vec::new(),
            negative: Vec::new(),
        }
    }
}

/// Input for creating one content rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleCreateInput {
    /// Optional stable rule ID. When absent, storage derives one from the rule.
    pub id: Option<String>,
    /// Rule lifecycle status. Defaults to `draft`.
    pub status: Option<String>,
    /// Rule priority. Lower numbers run earlier.
    pub priority: Option<i64>,
    /// Short human-readable rule name.
    pub title: String,
    /// Agent-facing instruction text.
    pub instruction: String,
    /// Source that created this rule, such as `user` or `generated`.
    pub created_source: String,
    /// Positive and negative examples for the rule.
    pub examples: RuleExamples,
}

/// Input for updating one content rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleUpdateInput {
    /// Stable rule ID to update.
    pub id: String,
    /// Optional replacement lifecycle status.
    pub status: Option<String>,
    /// Optional replacement priority.
    pub priority: Option<i64>,
    /// Optional replacement title.
    pub title: Option<String>,
    /// Optional replacement instruction.
    pub instruction: Option<String>,
    /// Source making this update.
    pub source: String,
    /// Optional replacement positive examples.
    pub positive_examples: Option<Vec<String>>,
    /// Optional replacement negative examples.
    pub negative_examples: Option<Vec<String>>,
}

/// Input for changing a rule lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleStatusInput {
    /// Stable rule ID to update.
    pub id: String,
    /// New lifecycle status, such as `active`, `disabled`, or `archived`.
    pub status: String,
    /// Source making this status change.
    pub source: String,
}

/// Query options for testing one rule against stored content.
#[derive(Debug, Clone)]
pub struct RuleValidationQuery {
    /// Maximum number of matching rows to return.
    pub limit: usize,
    /// Number of matching rows to skip.
    pub offset: usize,
}

/// Detailed rule view with recent audit events.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuleDetail {
    /// Stored rule.
    pub rule: ContentRule,
    /// Recent audit events for this rule.
    pub audit: Vec<RuleAuditEvent>,
}

/// Append-only audit event for one content rule.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuleAuditEvent {
    /// Database row ID for this event.
    pub id: i64,
    /// Stable rule ID.
    pub rule_id: String,
    /// Event kind, such as `created`, `updated`, or `enabled`.
    pub event_kind: String,
    /// Source that caused the event.
    pub source: String,
    /// Event creation timestamp.
    pub created_at_unix_ms: i64,
    /// Snapshot of the rule after the event.
    pub snapshot: Value,
}

/// Page of stored posts that likely match one rule.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuleValidationPage {
    /// Rule being tested.
    pub rule: ContentRule,
    /// Number of stored content rows considered.
    pub total_stored: usize,
    /// Number of likely matching rows before pagination.
    pub total_matching: usize,
    /// Maximum number of rows requested.
    pub limit: usize,
    /// Number of matching rows skipped.
    pub offset: usize,
    /// Matching stored posts.
    pub items: Vec<RuleValidationMatch>,
}

/// One stored post that likely matches a rule.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuleValidationMatch {
    /// Stored content row.
    pub content: StoredContentItem,
    /// Heuristic score used to sort matches.
    pub score: usize,
    /// Rule terms found in the content text.
    pub matched_terms: Vec<String>,
    /// Positive examples found in the content text.
    pub matched_examples: Vec<String>,
}

/// Query options for deriving draft rule suggestions from feedback.
#[derive(Debug, Clone)]
pub struct RuleSuggestionQuery {
    /// Minimum active feedback examples required for one suggestion.
    pub min_feedback: usize,
    /// Maximum number of suggestions to return.
    pub limit: usize,
    /// Number of suggestions to skip.
    pub offset: usize,
}

/// Page of draft rule suggestions derived from user feedback.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuleSuggestionPage {
    /// Number of matching suggestions before pagination.
    pub total_matching: usize,
    /// Maximum number of suggestions requested.
    pub limit: usize,
    /// Number of suggestions skipped.
    pub offset: usize,
    /// Candidate rules.
    pub items: Vec<RuleSuggestion>,
}

/// Draft rule candidate derived from active feedback.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuleSuggestion {
    /// Proposed stable rule ID.
    pub id: String,
    /// Proposed rule lifecycle status.
    pub status: String,
    /// Proposed priority.
    pub priority: i64,
    /// Proposed title.
    pub title: String,
    /// Proposed instruction.
    pub instruction: String,
    /// Proposed source.
    pub source: String,
    /// Number of active feedback records supporting this suggestion.
    pub feedback_count: usize,
    /// Feedback reasons that produced this suggestion.
    pub reasons: Vec<String>,
    /// Proposed examples.
    pub examples: RuleExamples,
    /// Feedback rows used as evidence.
    pub evidence: Vec<XDislikedPost>,
}

impl ContentStore {
    /// Opens the per-site storage databases under the configured data directory.
    pub fn from_env() -> Result<Self> {
        Self::with_data_dir(data_dir_from_env()?)
    }

    pub(crate) fn with_data_dir(data_dir: impl AsRef<Path>) -> Result<Self> {
        let x_com_path = site_db_path(data_dir.as_ref(), x_com::SITE_DIR);
        if env_flag_default(RESET_X_DB_ENV, false) {
            reset_sqlite_db_files(&x_com_path)?;
            warn!(
                site = x_com::SITE_DIR,
                path = %x_com_path.display(),
                env = RESET_X_DB_ENV,
                "reset site storage database before opening"
            );
        }

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

    /// Stores user feedback with feedback-time rule context.
    pub fn record_x_feedback_with_context(
        &self,
        item: &ContentItem,
        feedback: FeedbackKind,
        reason: &str,
        feedback_context: &FeedbackContext,
    ) -> Result<bool> {
        let mut db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.record_feedback_with_context(item, feedback, reason, feedback_context)
    }

    /// Returns the current feedback state for one X/Twitter content item.
    pub fn x_feedback_state(&self, item: &ContentItem) -> Result<Option<XFeedbackState>> {
        let db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.feedback_state(item)
    }

    /// Lists stored X/Twitter dislikes for agent and CLI inspection.
    pub fn x_dislikes(&self, query: XDislikeQuery) -> Result<XDislikePage> {
        let db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.dislikes(query)
    }

    /// Lists stored X/Twitter content rules.
    pub fn x_rules(&self, query: RuleQuery) -> Result<RulePage> {
        let db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.rules(query)
    }

    /// Loads one X/Twitter content rule with recent audit events.
    pub fn x_rule_detail(&self, id: &str) -> Result<Option<RuleDetail>> {
        let db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.rule_detail(id)
    }

    /// Creates one X/Twitter content rule.
    pub fn x_create_rule(&self, input: RuleCreateInput) -> Result<ContentRule> {
        let mut db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.create_rule(input)
    }

    /// Updates one X/Twitter content rule.
    pub fn x_update_rule(&self, input: RuleUpdateInput) -> Result<Option<ContentRule>> {
        let mut db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.update_rule(input)
    }

    /// Changes one X/Twitter content rule status.
    pub fn x_update_rule_status(&self, input: RuleStatusInput) -> Result<Option<ContentRule>> {
        let mut db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.update_rule_status(input)
    }

    /// Tests one X/Twitter rule against stored content.
    pub fn x_validate_rule(
        &self,
        id: &str,
        query: RuleValidationQuery,
    ) -> Result<Option<RuleValidationPage>> {
        let db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.validate_rule(id, query)
    }

    /// Suggests draft X/Twitter rules from active feedback.
    pub fn x_rule_suggestions(&self, query: RuleSuggestionQuery) -> Result<RuleSuggestionPage> {
        let db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.rule_suggestions(query)
    }

    /// Returns aggregate counts for stored X/Twitter content.
    pub fn x_content_stats(&self) -> Result<ContentStats> {
        let db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.content_stats()
    }

    /// Lists or searches stored X/Twitter content.
    pub fn x_content(&self, query: ContentQuery) -> Result<ContentPage> {
        let db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.content(query)
    }

    /// Creates or updates an annotation for stored X/Twitter content.
    pub fn x_upsert_content_annotation(
        &self,
        input: ContentAnnotationInput,
    ) -> Result<ContentAnnotation> {
        let mut db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.upsert_content_annotation(input)
    }

    /// Lists annotations for stored X/Twitter content.
    pub fn x_content_annotations(
        &self,
        query: ContentAnnotationQuery,
    ) -> Result<ContentAnnotationPage> {
        let db = self
            .x_com
            .lock()
            .expect("X storage mutex should not be poisoned");
        db.content_annotations(query)
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
        return Ok(PathBuf::from(home).join(".local/share/weblayer"));
    }

    Err(StorageError::MissingDataDir)
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_flag_default(name: &str, default: bool) -> bool {
    std::env::var(name)
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(default)
}

fn reset_sqlite_db_files(path: &Path) -> Result<()> {
    remove_file_if_exists(path)?;
    remove_file_if_exists(&sqlite_sidecar_path(path, "-wal"))?;
    remove_file_if_exists(&sqlite_sidecar_path(path, "-shm"))?;
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let Some(file_name) = path.file_name() else {
        return PathBuf::from(format!("{}{suffix}", path.display()));
    };

    path.with_file_name(format!("{}{suffix}", file_name.to_string_lossy()))
}

type Result<T> = std::result::Result<T, StorageError>;

/// Error returned by the filesystem-backed content store.
#[derive(Debug)]
pub enum StorageError {
    /// No data directory could be determined from `WEBLAYER_DATA_DIR` or `HOME`.
    MissingDataDir,
    /// Filesystem setup failed.
    Io(std::io::Error),
    /// SQLite failed to open, migrate, or write.
    Sqlite(rusqlite::Error),
    /// Payload JSON serialization failed.
    Json(serde_json::Error),
    /// Caller supplied invalid storage input.
    InvalidInput(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDataDir => write!(
                formatter,
                "missing data directory; set WEBLAYER_DATA_DIR or HOME"
            ),
            Self::Io(error) => write!(formatter, "filesystem error: {error}"),
            Self::Sqlite(error) => write!(formatter, "sqlite error: {error}"),
            Self::Json(error) => write!(formatter, "json error: {error}"),
            Self::InvalidInput(message) => formatter.write_str(message),
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
        let path = std::env::temp_dir().join(format!(
            "weblayer-storage-test-{name}-{}",
            std::process::id()
        ));
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

    #[test]
    fn reset_sqlite_db_files_removes_database_and_wal_sidecars() {
        let data_dir = temp_data_dir("reset-sqlite-files");
        let db_path = data_dir.join("x.com/db.sqlite");
        std::fs::create_dir_all(db_path.parent().unwrap()).expect("parent should be created");
        std::fs::write(&db_path, b"db").expect("db should be written");
        std::fs::write(sqlite_sidecar_path(&db_path, "-wal"), b"wal")
            .expect("wal should be written");
        std::fs::write(sqlite_sidecar_path(&db_path, "-shm"), b"shm")
            .expect("shm should be written");

        reset_sqlite_db_files(&db_path).expect("reset should remove files");

        assert!(!db_path.exists());
        assert!(!sqlite_sidecar_path(&db_path, "-wal").exists());
        assert!(!sqlite_sidecar_path(&db_path, "-shm").exists());

        let _ = std::fs::remove_dir_all(data_dir);
    }
}
