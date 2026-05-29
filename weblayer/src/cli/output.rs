use super::{CliError, CliResult};
use serde_json::Value;

pub(super) fn print_json(value: &Value) -> CliResult<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub(super) fn print_rules(value: &Value) {
    print_page_header("rules", value);
    println!("{:<36} {:<10} {:>8}  TITLE", "ID", "STATUS", "PRIORITY");
    for item in value_items(value) {
        println!(
            "{:<36} {:<10} {:>8}  {}",
            truncate(value_str(item, "id").unwrap_or(""), 36),
            value_str(item, "status").unwrap_or(""),
            value_i64(item, "priority")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            value_str(item, "title").unwrap_or("")
        );
    }
}

pub(super) fn print_rule_detail(value: &Value) {
    let rule = value.get("rule").unwrap_or(&Value::Null);
    println!("rule: {}", value_str(rule, "id").unwrap_or(""));
    println!("site: {}", value_str(value, "site").unwrap_or(""));
    println!("status: {}", value_str(rule, "status").unwrap_or(""));
    println!(
        "priority: {}",
        value_i64(rule, "priority")
            .map(|value| value.to_string())
            .unwrap_or_default()
    );
    println!("title: {}", value_str(rule, "title").unwrap_or(""));
    println!("source: {}", value_str(rule, "createdSource").unwrap_or(""));
    println!(
        "updated: {}",
        value_i64(rule, "updatedAtUnixMs")
            .map(|value| value.to_string())
            .unwrap_or_default()
    );
    println!(
        "instruction: {}",
        value_str(rule, "instruction").unwrap_or("")
    );

    let examples = rule.get("examples").unwrap_or(&Value::Null);
    print_string_array("positive examples", examples.get("positive"));
    print_string_array("negative examples", examples.get("negative"));

    let audit = value
        .get("audit")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if !audit.is_empty() {
        println!("audit:");
        for event in audit.iter().take(10) {
            println!(
                "  {:<12} {:<18} {}",
                value_str(event, "eventKind").unwrap_or(""),
                value_str(event, "source").unwrap_or(""),
                value_i64(event, "createdAtUnixMs")
                    .map(|value| value.to_string())
                    .unwrap_or_default()
            );
        }
    }
}

pub(super) fn print_rule_saved(value: &Value, action: &str) {
    let rule = value.get("rule").unwrap_or(&Value::Null);
    println!(
        "rule {} {} ({}, priority {})",
        value_str(rule, "id").unwrap_or("unknown"),
        action,
        value_str(rule, "status").unwrap_or("unknown"),
        value_i64(rule, "priority")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".into())
    );
}

pub(super) fn print_rule_validation(value: &Value) {
    let rule = value.get("rule").unwrap_or(&Value::Null);
    println!(
        "rule validation for {}: {} likely matches from {} stored items, limit {}, offset {}",
        value_str(rule, "id").unwrap_or(""),
        value_usize(value, "totalMatching").unwrap_or(0),
        value_usize(value, "totalStored").unwrap_or(0),
        value_usize(value, "limit").unwrap_or(0),
        value_usize(value, "offset").unwrap_or(0)
    );
    println!(
        "{:<28} {:<18} {:>5}  {:<24}  TEXT",
        "STORAGE KEY", "AUTHOR", "SCORE", "MATCHES"
    );
    for item in value_items(value) {
        let content = item.get("content").unwrap_or(&Value::Null);
        let matched_terms = value_string_array(item, "matchedTerms").join(",");
        let matched_examples = value_string_array(item, "matchedExamples").join(",");
        let matches = if matched_examples.is_empty() {
            matched_terms
        } else if matched_terms.is_empty() {
            matched_examples
        } else {
            format!("{matched_terms};{matched_examples}")
        };
        println!(
            "{:<28} {:<18} {:>5}  {:<24}  {}",
            truncate(value_str(content, "storageKey").unwrap_or(""), 28),
            truncate(value_str(content, "author").unwrap_or(""), 18),
            value_usize(item, "score")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            truncate(&matches, 24),
            truncate(value_str(content, "text").unwrap_or(""), 72)
        );
    }
}

pub(super) fn print_rule_suggestions(value: &Value) {
    print_page_header("rule suggestions", value);
    println!("{:<36} {:>8} {:>8}  TITLE", "ID", "FEEDBACK", "PRIORITY");
    for item in value_items(value) {
        println!(
            "{:<36} {:>8} {:>8}  {}",
            truncate(value_str(item, "id").unwrap_or(""), 36),
            value_usize(item, "feedbackCount")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            value_i64(item, "priority")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            value_str(item, "title").unwrap_or("")
        );
        println!(
            "  instruction: {}",
            value_str(item, "instruction").unwrap_or("")
        );
        let examples = item.get("examples").unwrap_or(&Value::Null);
        print_string_array("  positive examples", examples.get("positive"));
    }
}

pub(super) fn print_rule_set_proposals(value: &Value) {
    print_page_header("rule proposals", value);
    println!(
        "{:<36} {:<10} {:<24} {:>8} {:>8} {:>8}",
        "ID", "STATUS", "SOURCE", "FEEDBACK", "RULES", "CHANGES"
    );
    for item in value_items(value) {
        println!(
            "{:<36} {:<10} {:<24} {:>8} {:>8} {:>8}",
            truncate(value_str(item, "id").unwrap_or(""), 36),
            value_str(item, "status").unwrap_or(""),
            truncate(value_str(item, "source").unwrap_or(""), 24),
            value_usize(item, "feedbackCount")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            value_usize(item, "activeRuleCount")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            item.get("changes")
                .and_then(Value::as_array)
                .map(|changes| changes.len().to_string())
                .unwrap_or_default()
        );
    }
}

pub(super) fn print_rule_set_proposal(value: &Value) {
    let proposal = value.get("proposal").unwrap_or(value);
    println!("proposal: {}", value_str(proposal, "id").unwrap_or(""));
    println!("site: {}", value_str(value, "site").unwrap_or(""));
    println!("status: {}", value_str(proposal, "status").unwrap_or(""));
    println!("source: {}", value_str(proposal, "source").unwrap_or(""));
    println!(
        "feedback: {}, active rules: {}",
        value_usize(proposal, "feedbackCount").unwrap_or(0),
        value_usize(proposal, "activeRuleCount").unwrap_or(0)
    );

    let changes = proposal
        .get("changes")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if changes.is_empty() {
        println!("changes: none");
        return;
    }

    println!("changes:");
    for change in changes {
        let action = value_str(change, "action").unwrap_or("");
        let rule_id = value_str(change, "ruleId").unwrap_or("");
        let title = value_str(change, "title").unwrap_or("");
        println!("  - {action} {rule_id}");
        if !title.is_empty() {
            println!("    title: {title}");
        }
        if let Some(priority) = value_i64(change, "priority") {
            println!("    priority: {priority}");
        }
        if let Some(status) = value_str(change, "status") {
            println!("    status: {status}");
        }
        println!(
            "    rationale: {}",
            value_str(change, "rationale").unwrap_or("")
        );
        let evidence = value_string_array(change, "evidenceStorageKeys");
        if !evidence.is_empty() {
            println!("    evidence: {}", evidence.join(", "));
        }
        let examples = change.get("examples").unwrap_or(&Value::Null);
        print_string_array("    positive examples", examples.get("positive"));
    }
}

pub(super) fn print_content(value: &Value) {
    print_page_header("content", value);
    println!("{:<28} {:<18} {:>5}  TEXT", "STORAGE KEY", "AUTHOR", "SEEN");
    for item in value_items(value) {
        println!(
            "{:<28} {:<18} {:>5}  {}",
            truncate(value_str(item, "storageKey").unwrap_or(""), 28),
            truncate(value_str(item, "author").unwrap_or(""), 18),
            value_i64(item, "seenCount")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            truncate(value_str(item, "text").unwrap_or(""), 88)
        );
    }
}

pub(super) fn print_content_stats(value: &Value) {
    let stats = value.get("stats").unwrap_or(&Value::Null);
    println!("site: {}", value_str(value, "site").unwrap_or(""));
    println!(
        "content kind: {}",
        value_str(stats, "contentKind").unwrap_or("")
    );
    println!(
        "unique items: {}",
        value_usize(stats, "uniqueItems").unwrap_or(0)
    );
    println!(
        "total encounters: {}",
        value_usize(stats, "totalEncounters").unwrap_or(0)
    );
    println!(
        "items with stable id: {}",
        value_usize(stats, "itemsWithStableId").unwrap_or(0)
    );
    println!(
        "first seen: {}",
        value_i64(stats, "firstSeenAtUnixMs")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".into())
    );
    println!(
        "last seen: {}",
        value_i64(stats, "lastSeenAtUnixMs")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".into())
    );
}

pub(super) fn print_feedback(value: &Value) {
    print_page_header("feedback", value);
    println!(
        "{:<28} {:<18} {:<8}  REASON",
        "STORAGE KEY", "AUTHOR", "ACTIVE"
    );
    for item in value_items(value) {
        println!(
            "{:<28} {:<18} {:<8}  {}",
            truncate(value_str(item, "storageKey").unwrap_or(""), 28),
            truncate(value_str(item, "author").unwrap_or(""), 18),
            value_bool(item, "active")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            truncate(value_str(item, "reason").unwrap_or(""), 88)
        );
    }
}

pub(super) fn print_annotations(value: &Value) {
    print_page_header("annotations", value);
    println!(
        "{:>5} {:<24} {:<12} {:<16} {:<18}  VALUE",
        "ID", "STORAGE KEY", "TYPE", "KEY", "SOURCE"
    );
    for item in value_items(value) {
        println!(
            "{:>5} {:<24} {:<12} {:<16} {:<18}  {}",
            value_i64(item, "id")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            truncate(value_str(item, "storageKey").unwrap_or(""), 24),
            truncate(value_str(item, "annotationType").unwrap_or(""), 12),
            truncate(value_str(item, "key").unwrap_or(""), 16),
            truncate(value_str(item, "source").unwrap_or(""), 18),
            truncate(
                &item
                    .get("value")
                    .map(Value::to_string)
                    .unwrap_or_else(|| "null".into()),
                72,
            )
        );
    }
}

pub(super) fn print_annotation_put(value: &Value) {
    let annotation = value.get("annotation").unwrap_or(&Value::Null);
    println!(
        "annotation {} upserted for {}",
        value_i64(annotation, "id")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".into()),
        value_str(annotation, "storageKey").unwrap_or("unknown")
    );
}

pub(super) fn parse_json_value(text: &str) -> CliResult<Value> {
    serde_json::from_str(text).map_err(|error| {
        CliError::message(format!(
            "annotation value must be valid JSON: {error}. Example: --value '\"local-ai\"'"
        ))
    })
}

pub(super) fn value_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn print_page_header(label: &str, value: &Value) {
    let site = value_str(value, "site").unwrap_or("");
    let total = value_usize(value, "totalMatching").unwrap_or(0);
    let limit = value_usize(value, "limit").unwrap_or(0);
    let offset = value_usize(value, "offset").unwrap_or(0);
    println!("{label} for {site}: total {total}, limit {limit}, offset {offset}");
}

fn value_items(value: &Value) -> &[Value] {
    value
        .get("items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn value_i64(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

fn value_usize(value: &Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn value_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn value_string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn print_string_array(label: &str, value: Option<&Value>) {
    let items = value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if items.is_empty() {
        println!("{label}: none");
        return;
    }

    println!("{label}:");
    for item in items {
        if let Some(text) = item.as_str() {
            println!("  - {text}");
        }
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_json_value_requires_json() {
        assert!(parse_json_value("plain text").is_err());
        assert_eq!(
            parse_json_value("\"plain text\"").unwrap(),
            json!("plain text")
        );
    }
}
