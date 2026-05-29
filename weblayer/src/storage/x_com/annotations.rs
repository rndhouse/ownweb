use super::super::Result;
use super::{clean_optional, now_unix_ms, sqlite_limit, Store};
use crate::storage::{
    ContentAnnotation, ContentAnnotationInput, ContentAnnotationPage, ContentAnnotationQuery,
};
use rusqlite::{params, Row};
use tracing::debug;

impl Store {
    pub(in crate::storage) fn upsert_content_annotation(
        &mut self,
        input: ContentAnnotationInput,
    ) -> Result<ContentAnnotation> {
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

    pub(in crate::storage) fn content_annotations(
        &self,
        query: ContentAnnotationQuery,
    ) -> Result<ContentAnnotationPage> {
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
