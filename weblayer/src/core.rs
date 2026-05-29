use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A batch of content items submitted for daemon-side analysis.
#[derive(Debug, Clone)]
pub struct AnalysisBatch {
    /// Site or integration source that produced the content.
    pub source: String,
    /// Normalized content items to classify.
    pub items: Vec<ContentItem>,
}

impl AnalysisBatch {
    /// Builds a normalized analysis batch.
    pub fn new(source: impl Into<String>, items: Vec<ContentItem>) -> Self {
        Self {
            source: source.into(),
            items,
        }
    }
}

/// A normalized item that can be analyzed by site-specific handlers.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentItem {
    /// Client-generated ID used to match daemon decisions to browser elements.
    pub client_id: String,
    /// Site-specific stable content ID when one is available.
    #[serde(default)]
    pub content_id: Option<String>,
    /// Canonical URL for the item when one is available.
    #[serde(default)]
    pub url: Option<String>,
    /// Site-specific author identifier when one is available.
    #[serde(default)]
    pub author: Option<String>,
    /// User-visible text extracted by the browser extension.
    #[serde(default)]
    pub text: String,
    /// Client-side capture timestamp.
    #[serde(default)]
    pub captured_at: Option<String>,
    /// Site-specific item kind such as `post`, `comment`, or `profile`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Extra site-specific fields that should remain outside the core schema.
    #[serde(default)]
    pub metadata: Value,
}

/// A browser page snapshot submitted for daemon-side interpretation.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomAnalysisBatch {
    /// Snapshot metadata for the live page.
    pub page: PageSnapshot,
    /// Candidate DOM regions captured by the browser extension.
    #[serde(default)]
    pub elements: Vec<DomElementSnapshot>,
}

/// Metadata for the page that produced a DOM snapshot.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageSnapshot {
    /// Browser URL at capture time.
    pub url: String,
    /// Browser document title at capture time.
    #[serde(default)]
    pub title: Option<String>,
    /// Client-side capture timestamp.
    #[serde(default)]
    pub captured_at: Option<String>,
}

/// A raw-ish DOM region snapshot captured by the browser extension.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomElementSnapshot {
    /// Client-generated ID used only to target the current live DOM region.
    pub client_id: String,
    /// Best-effort CSS selector for the region.
    #[serde(default)]
    pub selector: Option<String>,
    /// Lowercase HTML tag name for the region root.
    #[serde(default)]
    pub tag_name: Option<String>,
    /// ARIA role for the region root when present.
    #[serde(default)]
    pub role: Option<String>,
    /// Visible text captured from the region.
    #[serde(default)]
    pub text: String,
    /// Sanitized outer HTML captured from the region.
    #[serde(default)]
    pub html: Option<String>,
    /// Root attributes captured from the region.
    #[serde(default)]
    pub attributes: Vec<DomAttribute>,
    /// Links captured inside the region.
    #[serde(default)]
    pub links: Vec<DomLink>,
    /// Client-side hash of the captured region contents.
    #[serde(default)]
    pub snapshot_hash: Option<String>,
    /// Client-side capture timestamp.
    #[serde(default)]
    pub captured_at: Option<String>,
}

/// User feedback signal sent by the browser extension for a DOM region.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FeedbackKind {
    /// User requested less content like this region.
    ThumbsDown,
    /// User removed a previous thumbs-down signal for this region.
    UndoThumbsDown,
    /// User updated the reason attached to a thumbs-down signal.
    UpdateReason,
}

/// One DOM attribute captured from a region root.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomAttribute {
    /// Attribute name.
    pub name: String,
    /// Attribute value.
    pub value: String,
}

/// One link captured from a DOM region.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomLink {
    /// Absolute or page-relative href value.
    pub href: String,
    /// Visible link text.
    #[serde(default)]
    pub text: Option<String>,
    /// ARIA label when present.
    #[serde(default)]
    pub aria_label: Option<String>,
}

/// An action that the browser extension can apply to a content item.
#[derive(Debug, Clone, Copy, Serialize)]
#[allow(dead_code)]
#[serde(rename_all = "lowercase")]
pub enum DecisionAction {
    /// Leave the content unchanged.
    Keep,
    /// Hide the content from the page.
    Hide,
    /// Visually de-emphasize the content.
    Dim,
    /// Add a visible label without changing the content body.
    Label,
    /// Replace the content body with daemon-provided text.
    Replace,
}

impl DecisionAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Hide => "hide",
            Self::Dim => "dim",
            Self::Label => "label",
            Self::Replace => "replace",
        }
    }
}

/// Daemon output for one content item.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentDecision {
    /// Client-generated ID from the analyzed item.
    pub client_id: String,
    /// Browser-side action to apply.
    pub action: DecisionAction,
    /// Optional user-visible label.
    pub label: Option<String>,
    /// Optional internal or user-facing explanation.
    pub reason: Option<String>,
    /// Replacement body for `replace` actions.
    pub replacement_text: Option<String>,
    /// Classifier confidence on a `0.0..=1.0` scale when known.
    pub confidence: Option<f32>,
    /// Active rule IDs that contributed to this decision.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_rule_ids: Vec<String>,
}

impl ContentDecision {
    /// Creates a decision that leaves content unchanged.
    pub fn keep(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            action: DecisionAction::Keep,
            label: None,
            reason: None,
            replacement_text: None,
            confidence: Some(1.0),
            matched_rule_ids: Vec::new(),
        }
    }

    /// Creates a decision that hides content.
    pub fn hide(
        client_id: impl Into<String>,
        label: impl Into<String>,
        reason: impl Into<String>,
        confidence: f32,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            action: DecisionAction::Hide,
            label: Some(label.into()),
            reason: Some(reason.into()),
            replacement_text: None,
            confidence: Some(confidence),
            matched_rule_ids: Vec::new(),
        }
    }

    /// Adds rule IDs that contributed to this decision.
    pub fn with_matched_rule_ids(mut self, matched_rule_ids: Vec<String>) -> Self {
        self.matched_rule_ids = matched_rule_ids;
        self
    }
}

/// Feedback-time context used later for rule curation.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackContext {
    /// Active rules available to the decision agent when feedback controls were rendered.
    #[serde(default)]
    pub active_rules: Vec<FeedbackRuleContext>,
    /// Decision metadata for this item when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<FeedbackDecisionContext>,
}

/// Snapshot of one active rule in the feedback-time rule set.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackRuleContext {
    /// Stable rule ID.
    pub id: String,
    /// Rule priority at the time of feedback.
    pub priority: i64,
    /// Short human-readable title.
    pub title: String,
    /// Agent-facing instruction text.
    pub instruction: String,
    /// Rule update timestamp at the time of feedback.
    pub updated_at_unix_ms: i64,
    /// Positive examples attached to the rule.
    #[serde(default)]
    pub positive_examples: Vec<String>,
    /// Negative examples attached to the rule.
    #[serde(default)]
    pub negative_examples: Vec<String>,
}

/// Decision metadata available when the feedback control was rendered.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackDecisionContext {
    /// Browser-side decision action.
    pub action: String,
    /// Decision explanation when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Active rule IDs matched by the decision.
    #[serde(default)]
    pub matched_rule_ids: Vec<String>,
}

impl FeedbackContext {
    /// Returns a copy of this context with item-specific decision metadata attached.
    pub fn with_decision(&self, decision: &ContentDecision) -> Self {
        let mut context = self.clone();
        context.decision = Some(FeedbackDecisionContext {
            action: decision.action.as_str().into(),
            reason: decision.reason.clone(),
            matched_rule_ids: decision.matched_rule_ids.clone(),
        });
        context
    }
}

/// A command that the browser extension can apply to the live DOM.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomCommand {
    /// Browser-side action to apply.
    pub action: DomCommandAction,
    /// Targeting and validation data for the live DOM.
    pub target: DomCommandTarget,
    /// Optional user-visible label.
    pub label: Option<String>,
    /// Optional replacement text for `replaceText`.
    pub text: Option<String>,
    /// Optional explanation for diagnostics or tooltips.
    pub reason: Option<String>,
    /// Classifier confidence on a `0.0..=1.0` scale when known.
    pub confidence: Option<f32>,
    /// Rule and decision context to echo when the user provides feedback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback_context: Option<FeedbackContext>,
    /// Active rule IDs that contributed to this command.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_rule_ids: Vec<String>,
}

impl DomCommand {
    /// Builds a DOM command from a content decision and live DOM target.
    pub fn from_decision(decision: ContentDecision, target: DomCommandTarget) -> Self {
        let action = match decision.action {
            DecisionAction::Keep => DomCommandAction::Keep,
            DecisionAction::Hide => DomCommandAction::Hide,
            DecisionAction::Dim => DomCommandAction::Dim,
            DecisionAction::Label => DomCommandAction::InsertLabel,
            DecisionAction::Replace => DomCommandAction::ReplaceText,
        };

        Self {
            action,
            target,
            label: decision.label,
            text: decision.replacement_text,
            reason: decision.reason,
            confidence: decision.confidence,
            feedback_context: None,
            matched_rule_ids: decision.matched_rule_ids,
        }
    }

    /// Builds a command that installs a user-feedback control in the region.
    pub fn feedback_control(target: DomCommandTarget) -> Self {
        Self {
            action: DomCommandAction::InsertFeedbackControl,
            target,
            label: Some("Hide this post".into()),
            text: None,
            reason: Some("User feedback control".into()),
            confidence: None,
            feedback_context: None,
            matched_rule_ids: Vec::new(),
        }
    }

    /// Builds a feedback control that carries rule context for future feedback.
    pub fn feedback_control_with_context(
        target: DomCommandTarget,
        feedback_context: FeedbackContext,
    ) -> Self {
        let mut command = Self::feedback_control(target);
        command.feedback_context = Some(feedback_context);
        command
    }
}

/// Browser-side DOM operation names.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DomCommandAction {
    /// Remove WebLayer modifications and leave the region visible.
    Keep,
    /// Hide the region.
    Hide,
    /// Visually de-emphasize the region.
    Dim,
    /// Insert a label into the region.
    InsertLabel,
    /// Insert a user-feedback control into the region.
    InsertFeedbackControl,
    /// Replace the region text.
    ReplaceText,
}

/// Targeting and consistency checks for a DOM command.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomCommandTarget {
    /// Client-generated region ID from the snapshot.
    pub client_id: String,
    /// Best-effort selector fallback when the client ID is not currently mapped.
    pub selector: Option<String>,
    /// Snapshot hash that must still match before applying the command.
    pub must_match_snapshot_hash: Option<String>,
}
