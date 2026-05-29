use super::{error::ApiError, site::SiteScope, AppState};
use crate::storage::{XDislikeQuery, XDislikedPost};
use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

const DEFAULT_DISLIKE_LIMIT: usize = 100;
const MAX_DISLIKE_LIMIT: usize = 500;

pub(super) async fn feedback(
    State(state): State<AppState>,
    Query(query): Query<FeedbackQuery>,
) -> Result<Json<FeedbackResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let active = query.active.or(Some(true));
    let limit = query
        .limit
        .unwrap_or(DEFAULT_DISLIKE_LIMIT)
        .min(MAX_DISLIKE_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let page = match site {
        SiteScope::XCom => state.content_store.x_dislikes(XDislikeQuery {
            active,
            unprocessed: None,
            limit,
            offset,
        })?,
    };

    Ok(Json(FeedbackResponse {
        site: site.as_str(),
        active,
        total_matching: page.total_matching,
        limit: page.limit,
        offset: page.offset,
        items: page.items,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FeedbackQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
    /// Filter by current active feedback state. Defaults to active feedback.
    active: Option<bool>,
    /// Maximum number of rows to return. Defaults to 100 and is capped at 500.
    limit: Option<usize>,
    /// Number of matching rows to skip.
    offset: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FeedbackResponse {
    site: &'static str,
    active: Option<bool>,
    total_matching: usize,
    limit: usize,
    offset: usize,
    items: Vec<XDislikedPost>,
}
