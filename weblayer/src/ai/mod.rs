mod codex_app;
mod summary_cache;

use crate::{core::ContentItem, storage::ContentRule};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use summary_cache::SummaryCache;
use tracing::{debug, warn, Level};

const CODEX_ENABLED_ENV: &str = "WEBLAYER_CODEX_APP_ENABLED";

/// Shared AI analyzer used by site handlers.
#[derive(Clone)]
pub struct AiAnalyzer {
    codex_app: Option<Arc<codex_app::CodexAppAnalyzer>>,
    x_summary_cache: Arc<Mutex<SummaryCache>>,
}

impl AiAnalyzer {
    /// Builds the analyzer from local daemon environment variables.
    pub fn from_env() -> Self {
        let codex_enabled = env_flag_default(CODEX_ENABLED_ENV, true);

        let codex_app = codex_enabled.then(|| Arc::new(codex_app::CodexAppAnalyzer::from_env()));
        Self {
            codex_app,
            x_summary_cache: Arc::new(Mutex::new(SummaryCache::from_env())),
        }
    }

    /// Gets Codex opinions for X content.
    pub async fn x_opinions(
        &self,
        items: &[ContentItem],
        rules: &[AiContentRule],
    ) -> Option<Vec<AiOpinion>> {
        let mut opinions = Vec::new();
        let mut misses = Vec::new();
        let now = Instant::now();
        let rule_scope = rule_cache_scope(rules);

        {
            let mut cache = self
                .x_summary_cache
                .lock()
                .expect("X summary cache mutex should not be poisoned");

            for item in items {
                if let Some(hit) = cache.get(item, &rule_scope, now) {
                    opinions.push(hit);
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

        for item in &misses {
            debug_x_agent_query_item(item);
        }

        match codex_app.x_opinions(&misses, rules).await {
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
                            cache.insert(item, &rule_scope, opinion, now);
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

    /// Gets X opinions only when every requested item is already cached.
    pub fn cached_x_opinions(
        &self,
        items: &[ContentItem],
        rules: &[AiContentRule],
    ) -> Option<Vec<AiOpinion>> {
        let now = Instant::now();
        let rule_scope = rule_cache_scope(rules);
        let mut cache = self
            .x_summary_cache
            .lock()
            .expect("X summary cache mutex should not be poisoned");
        let mut opinions = Vec::with_capacity(items.len());

        for item in items {
            opinions.push(cache.get(item, &rule_scope, now)?);
        }

        Some(opinions)
    }

    #[cfg(test)]
    pub(crate) fn for_tests_with_x_summaries(summaries: &[(&ContentItem, &str, f32)]) -> Self {
        let mut cache = SummaryCache::from_env();
        let now = Instant::now();

        for (item, summary, confidence) in summaries {
            cache.insert(
                item,
                &rule_cache_scope(&[]),
                &AiOpinion {
                    client_id: item.client_id.clone(),
                    action: AiAction::Keep,
                    opinion: (*summary).into(),
                    confidence: *confidence,
                    matched_rule_ids: Vec::new(),
                },
                now,
            );
        }

        Self {
            codex_app: None,
            x_summary_cache: Arc::new(Mutex::new(cache)),
        }
    }
}

fn debug_x_agent_query_item(item: &ContentItem) {
    if !tracing::enabled!(target: "weblayer_daemon::ai", Level::DEBUG) {
        return;
    }

    if let Ok(post_json) = serde_json::to_string(item) {
        debug!(
            target: "weblayer_daemon::ai",
            client_id = item.client_id.as_str(),
            content_id = item.content_id.as_deref(),
            url = item.url.as_deref(),
            post = %post_json,
            "querying Codex app-server for X post"
        );
    }
}

fn env_flag_default(name: &str, default: bool) -> bool {
    std::env::var(name)
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(default)
}

/// Active user rule sent to the AI analyzer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiContentRule {
    /// Stable rule ID.
    pub id: String,
    /// Short human-readable rule name.
    pub title: String,
    /// Agent-facing instruction text.
    pub instruction: String,
    /// Examples that should match this rule.
    pub positive_examples: Vec<String>,
    /// Examples that should not match this rule.
    pub negative_examples: Vec<String>,
}

impl From<ContentRule> for AiContentRule {
    fn from(rule: ContentRule) -> Self {
        Self {
            id: rule.id,
            title: rule.title,
            instruction: rule.instruction,
            positive_examples: rule.examples.positive,
            negative_examples: rule.examples.negative,
        }
    }
}

/// AI opinion attached to one analyzed content item.
#[derive(Debug, Clone)]
pub struct AiOpinion {
    /// Client-generated ID from the analyzed content item.
    pub client_id: String,
    /// Rule-driven action to apply to the item.
    pub action: AiAction,
    /// Short opinion suitable for a browser label.
    pub opinion: String,
    /// Model confidence on a `0.0..=1.0` scale.
    pub confidence: f32,
    /// Active rule IDs that caused a hide decision.
    pub matched_rule_ids: Vec<String>,
}

/// AI action for an analyzed content item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiAction {
    /// Leave the item visible.
    Keep,
    /// Hide the item because it matches at least one active rule.
    Hide,
}

fn rule_cache_scope(rules: &[AiContentRule]) -> String {
    if rules.is_empty() {
        return "none".into();
    }

    let mut text = String::new();
    for rule in rules {
        text.push_str(&rule.id);
        text.push('\n');
        text.push_str(&rule.title);
        text.push('\n');
        text.push_str(&rule.instruction);
        text.push('\n');
        for example in &rule.positive_examples {
            text.push_str("+ ");
            text.push_str(example);
            text.push('\n');
        }
        for example in &rule.negative_examples {
            text.push_str("- ");
            text.push_str(example);
            text.push('\n');
        }
    }

    format!("{:016x}", stable_hash(&text))
}

fn stable_hash(text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;

    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }

    hash
}
