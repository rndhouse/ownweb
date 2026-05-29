use crate::core::{ContentItem, DomAnalysisBatch, DomCommandTarget, DomElementSnapshot};
use serde_json::json;
use tracing::{trace, Level};

pub(super) fn extract_items(batch: &DomAnalysisBatch) -> Vec<ExtractedItem> {
    let mut page_status_href = status_href_from_page(&batch.page.url);
    let mut extracted_items = Vec::new();

    for element in &batch.elements {
        let Some(extracted) = extract_item(batch, element, page_status_href.as_deref()) else {
            continue;
        };

        page_status_href = None;
        extracted_items.push(extracted);
    }

    extracted_items
}

fn extract_item(
    batch: &DomAnalysisBatch,
    element: &DomElementSnapshot,
    page_status_href: Option<&str>,
) -> Option<ExtractedItem> {
    if !has_post_region_evidence(element) {
        return None;
    }

    let status_href = find_status_href(element).or_else(|| page_status_href.map(ToOwned::to_owned));
    let post_id = status_href
        .as_deref()
        .and_then(x_status_id)
        .map(ToOwned::to_owned);
    post_id.as_ref()?;

    let author = status_href.as_deref().and_then(author_handle);
    let item = ContentItem {
        client_id: element.client_id.clone(),
        content_id: post_id,
        url: status_href,
        author,
        text: element.text.clone(),
        captured_at: element
            .captured_at
            .clone()
            .or_else(|| batch.page.captured_at.clone()),
        kind: Some("post".into()),
        metadata: json!({
            "pageUrl": batch.page.url,
            "pageTitle": batch.page.title,
            "selector": element.selector,
            "tagName": element.tag_name,
            "role": element.role,
            "snapshotHash": element.snapshot_hash,
        }),
    };
    trace_identified_post(&item);

    let target = DomCommandTarget {
        client_id: element.client_id.clone(),
        selector: element.selector.clone(),
        must_match_snapshot_hash: element.snapshot_hash.clone(),
    };

    Some(ExtractedItem { item, target })
}

fn has_post_region_evidence(element: &DomElementSnapshot) -> bool {
    element
        .tag_name
        .as_deref()
        .is_some_and(|tag_name| tag_name.eq_ignore_ascii_case("article"))
        || element
            .role
            .as_deref()
            .is_some_and(|role| role.eq_ignore_ascii_case("article"))
        || has_root_attribute(element, "data-testid", "tweet")
}

fn has_root_attribute(element: &DomElementSnapshot, name: &str, value: &str) -> bool {
    element.attributes.iter().any(|attribute| {
        attribute.name.eq_ignore_ascii_case(name) && attribute.value.eq_ignore_ascii_case(value)
    })
}

fn trace_identified_post(item: &ContentItem) {
    if !tracing::enabled!(target: "weblayer_daemon::sites::x_com", Level::TRACE) {
        return;
    }

    if let Ok(post_json) = serde_json::to_string(item) {
        trace!(
            target: "weblayer_daemon::sites::x_com",
            client_id = item.client_id.as_str(),
            content_id = item.content_id.as_deref(),
            url = item.url.as_deref(),
            post = %post_json,
            "identified X post"
        );
    }
}

fn find_status_href(element: &DomElementSnapshot) -> Option<String> {
    element
        .links
        .iter()
        .find_map(|link| x_status_id(&link.href).map(|_| link.href.clone()))
}

fn status_href_from_page(url: &str) -> Option<String> {
    x_status_id(url).map(|_| url.to_string())
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

fn author_handle(url: &str) -> Option<String> {
    let without_scheme = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
    let path = without_scheme.split_once('/')?.1;
    let handle = path.split_once("/status/")?.0.trim_matches('/');

    (!handle.is_empty()).then(|| format!("@{handle}"))
}

pub(super) struct ExtractedItem {
    pub(super) item: ContentItem,
    pub(super) target: DomCommandTarget,
}
