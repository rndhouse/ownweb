use crate::core::ContentItem;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tracing::debug;

const DEFAULT_MAX_ENTRIES: usize = 10_000;
const DEFAULT_TTL_SECS: u64 = 24 * 60 * 60;
const MAX_ENTRIES_ENV: &str = "OWNWEB_X_SUMMARY_CACHE_MAX_ENTRIES";
const TTL_SECS_ENV: &str = "OWNWEB_X_SUMMARY_CACHE_TTL_SECS";

/// In-memory cache for X post summaries returned by Codex.
#[derive(Debug)]
pub struct SummaryCache {
    entries: HashMap<String, CachedSummary>,
    max_entries: usize,
    ttl: Duration,
}

impl SummaryCache {
    /// Builds a summary cache from local daemon environment variables.
    pub fn from_env() -> Self {
        Self::new(
            env_usize_default(MAX_ENTRIES_ENV, DEFAULT_MAX_ENTRIES),
            env_duration_secs_default(TTL_SECS_ENV, DEFAULT_TTL_SECS),
        )
    }

    fn new(max_entries: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            ttl,
        }
    }

    /// Gets a cached summary for an item and rewrites it to the current client ID.
    pub fn get(&mut self, item: &ContentItem, now: Instant) -> Option<CachedSummaryHit> {
        let key = cache_key(item)?;
        let is_expired = self
            .entries
            .get(&key)
            .is_some_and(|entry| now.saturating_duration_since(entry.created_at) > self.ttl);

        if is_expired {
            self.entries.remove(&key);
            return None;
        }

        let entry = self.entries.get_mut(&key)?;
        entry.last_used = now;

        debug!(
            target: "ownweb_daemon::summary_cache",
            client_id = %item.client_id,
            content_id = item.content_id.as_deref(),
            cache_key = %key,
            "X summary cache hit"
        );

        Some(CachedSummaryHit {
            client_id: item.client_id.clone(),
            summary: entry.summary.clone(),
            confidence: entry.confidence,
        })
    }

    /// Stores a summary for an item when that item has a stable cache key.
    pub fn insert(
        &mut self,
        item: &ContentItem,
        summary: impl Into<String>,
        confidence: f32,
        now: Instant,
    ) {
        let Some(key) = cache_key(item) else {
            return;
        };

        self.evict_expired(now);

        if !self.entries.contains_key(&key) && self.entries.len() >= self.max_entries {
            self.evict_lru();
        }

        self.entries.insert(
            key,
            CachedSummary {
                summary: summary.into(),
                confidence,
                created_at: now,
                last_used: now,
            },
        );
    }

    fn evict_expired(&mut self, now: Instant) {
        self.entries
            .retain(|_, entry| now.saturating_duration_since(entry.created_at) <= self.ttl);
    }

    fn evict_lru(&mut self) {
        if let Some(key) = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone())
        {
            self.entries.remove(&key);
        }
    }
}

/// Cached summary rewritten for the current content item.
#[derive(Debug, Clone)]
pub struct CachedSummaryHit {
    /// Client-generated ID from the current analyzed content item.
    pub client_id: String,
    /// Cached summary text.
    pub summary: String,
    /// Cached model confidence.
    pub confidence: f32,
}

#[derive(Debug)]
struct CachedSummary {
    summary: String,
    confidence: f32,
    created_at: Instant,
    last_used: Instant,
}

fn cache_key(item: &ContentItem) -> Option<String> {
    let normalized_text = normalize_text(&item.text);

    if let Some(content_id) = stable_content_id(item) {
        return Some(format!(
            "x:v1:id:{}:text:{:016x}",
            content_id,
            stable_hash(&normalized_text)
        ));
    }

    let author = item.author.as_deref().unwrap_or_default().trim();
    let url = item.url.as_deref().map(normalize_url).unwrap_or_default();

    if author.is_empty() && url.is_empty() && normalized_text.is_empty() {
        return None;
    }

    Some(format!(
        "x:v1:fallback:{:016x}",
        stable_hash(&format!(
            "author={}\nurl={}\ntext={}",
            author, url, normalized_text
        ))
    ))
}

fn stable_content_id(item: &ContentItem) -> Option<String> {
    item.content_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            item.url
                .as_deref()
                .and_then(x_status_id)
                .map(ToOwned::to_owned)
        })
}

fn x_status_id(url: &str) -> Option<&str> {
    let marker = "/status/";
    let start = url.find(marker)? + marker.len();
    let rest = &url[start..];
    let end = rest
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(rest.len());

    (end > 0).then_some(&rest[..end])
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_url(url: &str) -> String {
    let trimmed = url.trim();
    let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);
    without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment)
        .to_string()
}

fn stable_hash(text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;

    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }

    hash
}

fn env_usize_default(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_duration_secs_default(name: &str, default_secs: u64) -> Duration {
    Duration::from_secs(
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value| *value > 0)
            .unwrap_or(default_secs),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn item(client_id: &str, content_id: Option<&str>, text: &str) -> ContentItem {
        ContentItem {
            client_id: client_id.into(),
            content_id: content_id.map(ToOwned::to_owned),
            url: Some("https://x.com/user/status/123?utm=1".into()),
            author: Some("@user".into()),
            text: text.into(),
            captured_at: None,
            kind: Some("post".into()),
            metadata: Value::Null,
        }
    }

    #[test]
    fn cache_hits_use_the_current_client_id() {
        let now = Instant::now();
        let mut cache = SummaryCache::new(10, Duration::from_secs(60));
        cache.insert(
            &item("first", Some("123"), "hello world"),
            "summary",
            0.7,
            now,
        );

        let hit = cache
            .get(&item("second", Some("123"), "hello world"), now)
            .expect("summary should be cached");

        assert_eq!(hit.client_id, "second");
        assert_eq!(hit.summary, "summary");
        assert_eq!(hit.confidence, 0.7);
    }

    #[test]
    fn cache_key_ignores_whitespace_variation() {
        let now = Instant::now();
        let mut cache = SummaryCache::new(10, Duration::from_secs(60));
        cache.insert(
            &item("first", Some("123"), "hello   world"),
            "summary",
            0.7,
            now,
        );

        assert!(cache
            .get(&item("second", Some("123"), "hello\nworld"), now)
            .is_some());
    }

    #[test]
    fn cache_key_changes_when_text_changes() {
        let now = Instant::now();
        let mut cache = SummaryCache::new(10, Duration::from_secs(60));
        cache.insert(
            &item("first", Some("123"), "hello world"),
            "summary",
            0.7,
            now,
        );

        assert!(cache
            .get(&item("second", Some("123"), "different text"), now)
            .is_none());
    }

    #[test]
    fn expired_entries_are_ignored() {
        let now = Instant::now();
        let mut cache = SummaryCache::new(10, Duration::from_secs(1));
        cache.insert(
            &item("first", Some("123"), "hello world"),
            "summary",
            0.7,
            now,
        );

        assert!(cache
            .get(
                &item("second", Some("123"), "hello world"),
                now + Duration::from_secs(2)
            )
            .is_none());
    }

    #[test]
    fn cache_evicts_least_recently_used_entries() {
        let now = Instant::now();
        let mut cache = SummaryCache::new(1, Duration::from_secs(60));
        cache.insert(&item("first", Some("123"), "one"), "first", 0.7, now);
        cache.insert(
            &item("second", Some("456"), "two"),
            "second",
            0.8,
            now + Duration::from_secs(1),
        );

        assert!(cache.get(&item("third", Some("123"), "one"), now).is_none());
        assert!(cache
            .get(&item("fourth", Some("456"), "two"), now)
            .is_some());
    }
}
