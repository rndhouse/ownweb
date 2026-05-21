use crate::core::{AnalysisBatch, ContentDecision};

pub mod x_com;

/// Dispatches an analysis batch to the matching site handler.
pub fn analyze(batch: &AnalysisBatch) -> Vec<ContentDecision> {
    match normalize_source(&batch.source).as_str() {
        "x.com" | "twitter.com" => x_com::analyze(&batch.items),
        _ => keep_all(batch),
    }
}

fn normalize_source(source: &str) -> String {
    source
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn keep_all(batch: &AnalysisBatch) -> Vec<ContentDecision> {
    batch
        .items
        .iter()
        .map(|item| ContentDecision::keep(item.client_id.clone()))
        .collect()
}
