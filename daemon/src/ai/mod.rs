mod codex_app;
mod summary_cache;

use crate::core::ContentItem;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use summary_cache::SummaryCache;
use tracing::warn;

/// Shared AI analyzer used by site handlers.
#[derive(Clone)]
pub struct AiAnalyzer {
    codex_app: Option<Arc<codex_app::CodexAppAnalyzer>>,
    x_summary_cache: Arc<Mutex<SummaryCache>>,
}

impl AiAnalyzer {
    /// Builds the analyzer from local daemon environment variables.
    pub fn from_env() -> Self {
        let codex_enabled = std::env::var("PAIRPILOT_CODEX_APP_ENABLED")
            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
            .unwrap_or(true);

        let codex_app = codex_enabled.then(|| Arc::new(codex_app::CodexAppAnalyzer::from_env()));
        Self {
            codex_app,
            x_summary_cache: Arc::new(Mutex::new(SummaryCache::from_env())),
        }
    }

    /// Gets Codex opinions for X content.
    pub async fn x_opinions(&self, items: &[ContentItem]) -> Option<Vec<AiOpinion>> {
        let mut opinions = Vec::new();
        let mut misses = Vec::new();
        let now = Instant::now();

        {
            let mut cache = self
                .x_summary_cache
                .lock()
                .expect("X summary cache mutex should not be poisoned");

            for item in items {
                if let Some(hit) = cache.get(item, now) {
                    opinions.push(AiOpinion {
                        client_id: hit.client_id,
                        opinion: hit.summary,
                        confidence: hit.confidence,
                    });
                } else {
                    misses.push(item.clone());
                }
            }
        }

        if misses.is_empty() {
            return Some(opinions);
        }

        let Some(codex_app) = self.codex_app.as_ref() else {
            return (!opinions.is_empty()).then_some(opinions);
        };

        match codex_app.x_opinions(&misses).await {
            Ok(fresh_opinions) => {
                let misses_by_client_id: HashMap<_, _> = misses
                    .iter()
                    .map(|item| (item.client_id.as_str(), item))
                    .collect();
                let now = Instant::now();

                {
                    let mut cache = self
                        .x_summary_cache
                        .lock()
                        .expect("X summary cache mutex should not be poisoned");

                    for opinion in &fresh_opinions {
                        if let Some(item) = misses_by_client_id.get(opinion.client_id.as_str()) {
                            cache.insert(item, &opinion.opinion, opinion.confidence, now);
                        }
                    }
                }

                opinions.extend(fresh_opinions);
                Some(opinions)
            }
            Err(error) => {
                warn!(%error, "codex app-server opinion unavailable");
                (!opinions.is_empty()).then_some(opinions)
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
