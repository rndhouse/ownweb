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
        }
    }

    /// Creates a decision that dims content.
    pub fn dim(
        client_id: impl Into<String>,
        label: impl Into<String>,
        reason: impl Into<String>,
        confidence: f32,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            action: DecisionAction::Dim,
            label: Some(label.into()),
            reason: Some(reason.into()),
            replacement_text: None,
            confidence: Some(confidence),
        }
    }

    /// Creates a decision that attaches a label to content.
    pub fn label(
        client_id: impl Into<String>,
        label: impl Into<String>,
        reason: impl Into<String>,
        confidence: f32,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            action: DecisionAction::Label,
            label: Some(label.into()),
            reason: Some(reason.into()),
            replacement_text: None,
            confidence: Some(confidence),
        }
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
        }
    }

    /// Builds a command that gates content while daemon analysis is still running.
    pub fn checking(target: DomCommandTarget) -> Self {
        Self {
            action: DomCommandAction::Dim,
            target,
            label: Some("OwnWeb: checking".into()),
            text: None,
            reason: Some("Waiting for local analysis".into()),
            confidence: None,
        }
    }
}

/// Browser-side DOM operation names.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DomCommandAction {
    /// Remove OwnWeb modifications and leave the region visible.
    Keep,
    /// Hide the region.
    Hide,
    /// Visually de-emphasize the region.
    Dim,
    /// Insert a label into the region.
    InsertLabel,
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
