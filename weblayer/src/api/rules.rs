use super::{
    error::ApiError,
    input::{clean_query_value, required_text},
    site::SiteScope,
    AppState,
};
use crate::storage::{
    ContentRule, RuleAuditEvent, RuleCreateInput, RuleExamples, RuleQuery, RuleStatusInput,
    RuleSuggestion, RuleSuggestionQuery, RuleUpdateInput, RuleValidationMatch,
};
use axum::{
    extract::{Path as AxumPath, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

const DEFAULT_CONTENT_LIMIT: usize = 100;
const MAX_CONTENT_LIMIT: usize = 500;
const DEFAULT_RULE_LIMIT: usize = 100;
const MAX_RULE_LIMIT: usize = 500;

pub(super) async fn rule_suggestions(
    State(state): State<AppState>,
    Query(query): Query<RuleSuggestionsQuery>,
) -> Result<Json<RuleSuggestionsResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_RULE_LIMIT)
        .min(MAX_RULE_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let min_feedback = query.min_feedback.unwrap_or(1).max(1);
    let page = match site {
        SiteScope::XCom => state
            .content_store
            .x_rule_suggestions(RuleSuggestionQuery {
                min_feedback,
                limit,
                offset,
            })?,
    };

    Ok(Json(RuleSuggestionsResponse {
        site: site.as_str(),
        min_feedback,
        total_matching: page.total_matching,
        limit: page.limit,
        offset: page.offset,
        items: page.items,
    }))
}

pub(super) async fn rules(
    State(state): State<AppState>,
    Query(query): Query<RulesQuery>,
) -> Result<Json<RulesResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let status = query
        .status
        .map(|status| status.trim().to_string())
        .filter(|status| !status.is_empty());
    let limit = query
        .limit
        .unwrap_or(DEFAULT_RULE_LIMIT)
        .min(MAX_RULE_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let page = match site {
        SiteScope::XCom => state.content_store.x_rules(RuleQuery {
            status: status.clone(),
            limit,
            offset,
        })?,
    };

    Ok(Json(RulesResponse {
        site: site.as_str(),
        status,
        total_matching: page.total_matching,
        limit: page.limit,
        offset: page.offset,
        items: page.items,
    }))
}

pub(super) async fn rule_detail(
    State(state): State<AppState>,
    AxumPath(rule_id): AxumPath<String>,
    Query(query): Query<RuleSiteQuery>,
) -> Result<Json<RuleDetailResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let detail = match site {
        SiteScope::XCom => state.content_store.x_rule_detail(&rule_id)?,
    }
    .ok_or_else(|| ApiError::not_found(format!("rule not found: {rule_id}")))?;

    Ok(Json(RuleDetailResponse {
        site: site.as_str(),
        rule: detail.rule,
        audit: detail.audit,
    }))
}

pub(super) async fn create_rule(
    State(state): State<AppState>,
    Query(query): Query<RuleSiteQuery>,
    Json(request): Json<CreateRuleRequest>,
) -> Result<Json<RuleMutationResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let title = required_text(request.title, "title")?;
    let instruction = required_text(request.instruction, "instruction")?;
    let created_source = clean_query_value(request.source).unwrap_or_else(|| "user".into());
    let examples = clean_rule_examples(request.examples);

    let rule = match site {
        SiteScope::XCom => state.content_store.x_create_rule(RuleCreateInput {
            id: clean_query_value(request.id),
            status: clean_query_value(request.status),
            priority: request.priority,
            title,
            instruction,
            created_source,
            examples,
        })?,
    };

    Ok(Json(RuleMutationResponse {
        site: site.as_str(),
        rule,
    }))
}

pub(super) async fn update_rule(
    State(state): State<AppState>,
    AxumPath(rule_id): AxumPath<String>,
    Query(query): Query<RuleSiteQuery>,
    Json(request): Json<UpdateRuleRequest>,
) -> Result<Json<RuleMutationResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let examples = request.examples.unwrap_or_default();
    let rule = match site {
        SiteScope::XCom => state.content_store.x_update_rule(RuleUpdateInput {
            id: rule_id.clone(),
            status: clean_query_value(request.status),
            priority: request.priority,
            title: clean_query_value(request.title),
            instruction: clean_query_value(request.instruction),
            source: clean_query_value(request.source).unwrap_or_else(|| "user".into()),
            positive_examples: examples.positive,
            negative_examples: examples.negative,
        })?,
    }
    .ok_or_else(|| ApiError::not_found(format!("rule not found: {rule_id}")))?;

    Ok(Json(RuleMutationResponse {
        site: site.as_str(),
        rule,
    }))
}

pub(super) async fn update_rule_status(
    State(state): State<AppState>,
    AxumPath(rule_id): AxumPath<String>,
    Query(query): Query<RuleSiteQuery>,
    Json(request): Json<UpdateRuleStatusRequest>,
) -> Result<Json<RuleMutationResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let status = required_text(request.status, "status")?;
    let source = clean_query_value(request.source).unwrap_or_else(|| "user".into());
    let rule = match site {
        SiteScope::XCom => state.content_store.x_update_rule_status(RuleStatusInput {
            id: rule_id.clone(),
            status,
            source,
        })?,
    }
    .ok_or_else(|| ApiError::not_found(format!("rule not found: {rule_id}")))?;

    Ok(Json(RuleMutationResponse {
        site: site.as_str(),
        rule,
    }))
}

pub(super) async fn validate_rule(
    State(state): State<AppState>,
    AxumPath(rule_id): AxumPath<String>,
    Query(query): Query<RuleValidationApiQuery>,
) -> Result<Json<RuleValidationResponse>, ApiError> {
    let site = SiteScope::from_param(query.site.as_deref())?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_CONTENT_LIMIT)
        .min(MAX_CONTENT_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let page = match site {
        SiteScope::XCom => state.content_store.x_validate_rule(
            &rule_id,
            crate::storage::RuleValidationQuery { limit, offset },
        )?,
    }
    .ok_or_else(|| ApiError::not_found(format!("rule not found: {rule_id}")))?;

    Ok(Json(RuleValidationResponse {
        site: site.as_str(),
        rule: page.rule,
        total_stored: page.total_stored,
        total_matching: page.total_matching,
        limit: page.limit,
        offset: page.offset,
        items: page.items,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RulesQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
    /// Optional rule status filter.
    status: Option<String>,
    /// Maximum number of rows to return. Defaults to 100 and is capped at 500.
    limit: Option<usize>,
    /// Number of matching rows to skip.
    offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RuleSuggestionsQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
    /// Minimum active feedback examples required for a suggestion.
    min_feedback: Option<usize>,
    /// Maximum number of suggestions to return. Defaults to 100 and is capped at 500.
    limit: Option<usize>,
    /// Number of suggestions to skip.
    offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RuleSiteQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RuleValidationApiQuery {
    /// Site scope for the request, such as `x.com`.
    site: Option<String>,
    /// Maximum number of likely matches to return. Defaults to 100 and is capped at 500.
    limit: Option<usize>,
    /// Number of matching rows to skip.
    offset: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RulesResponse {
    site: &'static str,
    status: Option<String>,
    total_matching: usize,
    limit: usize,
    offset: usize,
    items: Vec<ContentRule>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RuleSuggestionsResponse {
    site: &'static str,
    min_feedback: usize,
    total_matching: usize,
    limit: usize,
    offset: usize,
    items: Vec<RuleSuggestion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateRuleRequest {
    /// Optional stable rule ID.
    id: Option<String>,
    /// Optional lifecycle status. Defaults to `draft`.
    status: Option<String>,
    /// Optional priority. Lower numbers run earlier.
    priority: Option<i64>,
    /// Short human-readable title.
    title: String,
    /// Agent-facing instruction text.
    instruction: String,
    /// Source that created the rule. Defaults to `user`.
    source: Option<String>,
    /// Positive and negative examples.
    #[serde(default)]
    examples: RuleExamples,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct RuleExamplesPatch {
    /// Replacement positive examples when present.
    positive: Option<Vec<String>>,
    /// Replacement negative examples when present.
    negative: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateRuleRequest {
    /// Optional replacement lifecycle status.
    status: Option<String>,
    /// Optional replacement priority.
    priority: Option<i64>,
    /// Optional replacement title.
    title: Option<String>,
    /// Optional replacement instruction text.
    instruction: Option<String>,
    /// Source that updated the rule. Defaults to `user`.
    source: Option<String>,
    /// Optional examples patch. Present arrays replace only that example side.
    examples: Option<RuleExamplesPatch>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateRuleStatusRequest {
    /// New lifecycle status.
    status: String,
    /// Source that changed the status. Defaults to `user`.
    source: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RuleDetailResponse {
    site: &'static str,
    rule: ContentRule,
    audit: Vec<RuleAuditEvent>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RuleMutationResponse {
    site: &'static str,
    rule: ContentRule,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RuleValidationResponse {
    site: &'static str,
    rule: ContentRule,
    total_stored: usize,
    total_matching: usize,
    limit: usize,
    offset: usize,
    items: Vec<RuleValidationMatch>,
}

fn clean_rule_examples(examples: RuleExamples) -> RuleExamples {
    RuleExamples {
        positive: clean_rule_example_list(examples.positive),
        negative: clean_rule_example_list(examples.negative),
    }
}

fn clean_rule_example_list(examples: Vec<String>) -> Vec<String> {
    examples
        .into_iter()
        .map(|example| example.trim().to_string())
        .filter(|example| !example.is_empty())
        .collect()
}
