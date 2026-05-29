use super::{
    error::ApiError,
    input::{clean_query_value, required_text, validate_confidence},
    site::SiteScope,
    AppState,
};
use crate::storage::{
    ContentAnnotation, ContentAnnotationInput, ContentAnnotationQuery, ContentQuery, ContentStats,
    StoredContentItem,
};
use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_CONTENT_LIMIT: usize = 100;
const MAX_CONTENT_LIMIT: usize = 500;
const DEFAULT_ANNOTATION_LIMIT: usize = 100;
const MAX_ANNOTATION_LIMIT: usize = 500;

pub(super) async fn content(
    State(state): State<AppState>,
    Query(query): Query<ContentListQuery>,
) -> Result<Json<ContentListResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let search = query
        .q
        .map(|search| search.trim().to_string())
        .filter(|search| !search.is_empty());
    let limit = query
        .limit
        .unwrap_or(DEFAULT_CONTENT_LIMIT)
        .min(MAX_CONTENT_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let page = match site {
        SiteScope::XCom => state.content_store.x_content(ContentQuery {
            search: search.clone(),
            limit,
            offset,
        })?,
    };

    Ok(Json(ContentListResponse {
        site: site.as_str(),
        query: search,
        total_matching: page.total_matching,
        limit: page.limit,
        offset: page.offset,
        items: page.items,
    }))
}

pub(super) async fn content_stats(
    State(state): State<AppState>,
    Query(query): Query<ContentStatsQuery>,
) -> Result<Json<ContentStatsResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let stats = match site {
        SiteScope::XCom => state.content_store.x_content_stats()?,
    };

    Ok(Json(ContentStatsResponse {
        site: site.as_str(),
        stats,
    }))
}

pub(super) async fn content_annotations(
    State(state): State<AppState>,
    Query(query): Query<ContentAnnotationsQuery>,
) -> Result<Json<ContentAnnotationsResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let storage_key = clean_query_value(query.storage_key);
    let content_id = clean_query_value(query.content_id);
    let content_kind = clean_query_value(query.content_kind);
    let annotation_type = clean_query_value(query.annotation_type);
    let key = clean_query_value(query.key);
    let source = clean_query_value(query.source);
    let limit = query
        .limit
        .unwrap_or(DEFAULT_ANNOTATION_LIMIT)
        .min(MAX_ANNOTATION_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let page = match site {
        SiteScope::XCom => state
            .content_store
            .x_content_annotations(ContentAnnotationQuery {
                storage_key: storage_key.clone(),
                content_id: content_id.clone(),
                content_kind: content_kind.clone(),
                annotation_type: annotation_type.clone(),
                key: key.clone(),
                source: source.clone(),
                limit,
                offset,
            })?,
    };

    Ok(Json(ContentAnnotationsResponse {
        site: site.as_str(),
        storage_key,
        content_id,
        content_kind,
        annotation_type,
        key,
        source,
        total_matching: page.total_matching,
        limit: page.limit,
        offset: page.offset,
        items: page.items,
    }))
}

pub(super) async fn upsert_content_annotation(
    State(state): State<AppState>,
    Query(query): Query<ContentAnnotationSiteQuery>,
    Json(request): Json<UpsertContentAnnotationRequest>,
) -> Result<Json<UpsertContentAnnotationResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let storage_key = required_text(request.storage_key, "storageKey")?;
    let content_kind =
        clean_query_value(Some(request.content_kind)).unwrap_or_else(|| "post".into());
    let annotation_type = required_text(request.annotation_type, "annotationType")?;
    let key = request.key.trim().to_string();
    let source = required_text(request.source, "source")?;
    let confidence = validate_confidence(request.confidence)?;

    let annotation = match site {
        SiteScope::XCom => {
            state
                .content_store
                .x_upsert_content_annotation(ContentAnnotationInput {
                    storage_key,
                    content_kind,
                    annotation_type,
                    key,
                    value: request.value,
                    confidence,
                    source,
                })?
        }
    };

    Ok(Json(UpsertContentAnnotationResponse {
        site: site.as_str(),
        annotation,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContentListQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
    /// Optional full-text search query.
    q: Option<String>,
    /// Maximum number of rows to return. Defaults to 100 and is capped at 500.
    limit: Option<usize>,
    /// Number of matching rows to skip.
    offset: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContentListResponse {
    site: &'static str,
    query: Option<String>,
    total_matching: usize,
    limit: usize,
    offset: usize,
    items: Vec<StoredContentItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContentStatsQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContentStatsResponse {
    site: &'static str,
    stats: ContentStats,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContentAnnotationsQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
    /// Optional stable storage key filter.
    storage_key: Option<String>,
    /// Optional site-native content ID filter.
    content_id: Option<String>,
    /// Optional logical content kind filter.
    content_kind: Option<String>,
    /// Optional annotation category filter.
    annotation_type: Option<String>,
    /// Optional annotation key filter.
    key: Option<String>,
    /// Optional source filter.
    source: Option<String>,
    /// Maximum number of rows to return. Defaults to 100 and is capped at 500.
    limit: Option<usize>,
    /// Number of matching rows to skip.
    offset: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContentAnnotationsResponse {
    site: &'static str,
    storage_key: Option<String>,
    content_id: Option<String>,
    content_kind: Option<String>,
    annotation_type: Option<String>,
    key: Option<String>,
    source: Option<String>,
    total_matching: usize,
    limit: usize,
    offset: usize,
    items: Vec<ContentAnnotation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContentAnnotationSiteQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpsertContentAnnotationRequest {
    /// Stable storage key returned by content inspection endpoints.
    storage_key: String,
    /// Logical content kind. Defaults to `post`.
    #[serde(default = "default_content_kind")]
    content_kind: String,
    /// Annotation category, such as `tag`, `note`, or `topic`.
    annotation_type: String,
    /// Annotation key within its category.
    #[serde(default)]
    key: String,
    /// Arbitrary annotation payload.
    value: Value,
    /// Optional model confidence from 0.0 to 1.0.
    confidence: Option<f64>,
    /// Source that created or updated this annotation.
    source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpsertContentAnnotationResponse {
    site: &'static str,
    annotation: ContentAnnotation,
}

fn default_content_kind() -> String {
    "post".into()
}
