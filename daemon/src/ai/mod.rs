mod codex_app;

use crate::core::ContentItem;
use std::sync::Arc;

/// Shared AI analyzer used by site handlers.
#[derive(Clone)]
pub struct AiAnalyzer {
    codex_app: Option<Arc<codex_app::CodexAppAnalyzer>>,
}

impl AiAnalyzer {
    /// Builds the analyzer from local daemon environment variables.
    pub fn from_env() -> Self {
        let codex_enabled = std::env::var("PAIRPILOT_CODEX_APP_ENABLED")
            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
            .unwrap_or(true);

        let codex_app = codex_enabled.then(|| Arc::new(codex_app::CodexAppAnalyzer::from_env()));
        Self { codex_app }
    }

    /// Gets Codex opinions for X content.
    pub async fn x_opinions(&self, items: &[ContentItem]) -> Option<Vec<AiOpinion>> {
        let codex_app = self.codex_app.as_ref()?;

        match codex_app.x_opinions(items).await {
            Ok(opinions) => Some(opinions),
            Err(error) => {
                eprintln!("codex app-server opinion unavailable: {error}");
                None
            }
        }
    }
}

/// AI opinion attached to one analyzed content item.
#[derive(Debug, Clone)]
pub struct AiOpinion {
    /// Client-generated ID from the analyzed content item.
    pub client_id: String,
    /// Short opinion suitable for a browser label.
    pub opinion: String,
    /// Model confidence on a `0.0..=1.0` scale.
    pub confidence: f32,
}
