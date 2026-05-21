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
