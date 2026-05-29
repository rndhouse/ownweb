use super::super::Result;
use super::rules;
use rusqlite::Connection;

pub(super) fn initialize(connection: &Connection) -> Result<()> {
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
        payload_json TEXT NOT NULL,
        rule_context_json TEXT NOT NULL
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
        latest_payload_json TEXT NOT NULL,
        latest_rule_context_json TEXT NOT NULL
    );

    CREATE INDEX IF NOT EXISTS tweet_feedback_state_active_idx
        ON tweet_feedback_state(active);
    CREATE INDEX IF NOT EXISTS tweet_feedback_state_post_id_idx
        ON tweet_feedback_state(post_id);

    CREATE TABLE IF NOT EXISTS feedback_contexts (
        id TEXT PRIMARY KEY,
        site TEXT NOT NULL,
        created_at_unix_ms INTEGER NOT NULL,
        context_json TEXT NOT NULL
    );

    CREATE INDEX IF NOT EXISTS feedback_contexts_created_at_idx
        ON feedback_contexts(created_at_unix_ms);

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

    CREATE TABLE IF NOT EXISTS rule_set_proposals (
        id TEXT PRIMARY KEY,
        site TEXT NOT NULL,
        status TEXT NOT NULL,
        source TEXT NOT NULL,
        created_at_unix_ms INTEGER NOT NULL,
        proposal_json TEXT NOT NULL
    );

    CREATE INDEX IF NOT EXISTS rule_set_proposals_status_time_idx
        ON rule_set_proposals(status, created_at_unix_ms);

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
    migrate_feedback_context(connection)?;
    migrate_search(connection)?;
    rules::seed_default_rules(connection)?;

    Ok(())
}

fn migrate_feedback_context(connection: &Connection) -> Result<()> {
    ensure_column(
        connection,
        "tweet_feedback",
        "rule_context_json",
        "ALTER TABLE tweet_feedback ADD COLUMN rule_context_json TEXT NOT NULL DEFAULT '{\"activeRules\":[]}'",
    )?;
    ensure_column(
        connection,
        "tweet_feedback_state",
        "latest_rule_context_json",
        "ALTER TABLE tweet_feedback_state ADD COLUMN latest_rule_context_json TEXT NOT NULL DEFAULT '{\"activeRules\":[]}'",
    )?;

    Ok(())
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> Result<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !columns.iter().any(|existing| existing == column) {
        connection.execute(alter_sql, [])?;
    }

    Ok(())
}

fn migrate_search(connection: &Connection) -> Result<()> {
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
