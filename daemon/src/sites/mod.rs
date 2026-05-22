use crate::{
    ai::AiAnalyzer,
    core::{DomAnalysisBatch, DomCommand},
    storage::ContentStore,
};

pub mod x_com;

/// Dispatches a DOM snapshot to the matching site handler.
pub async fn analyze_dom(
    batch: &DomAnalysisBatch,
    ai_analyzer: &AiAnalyzer,
    content_store: &ContentStore,
) -> Vec<DomCommand> {
    match page_host(&batch.page.url).as_deref() {
        Some("x.com") | Some("twitter.com") => {
            x_com::analyze_dom(batch, ai_analyzer, content_store).await
        }
        _ => Vec::new(),
    }
}

fn page_host(url: &str) -> Option<String> {
    let without_scheme = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
    let host = without_scheme.split('/').next()?.split(':').next()?.trim();

    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}
