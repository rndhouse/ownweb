use super::super::{Result, StorageError};
use super::{
    clean_optional, content::content_item_from_row, normalize_text, now_unix_ms, sqlite_limit,
    stable_hash, Store, SITE_DIR,
};
use crate::storage::{
    ContentRule, RuleAuditEvent, RuleCreateInput, RuleDetail, RuleExamples, RulePage, RuleQuery,
    RuleSetProposal, RuleSetProposalAction, RuleSetProposalChange, RuleSetProposalCreateInput,
    RuleSetProposalPage, RuleSetProposalQuery, RuleStatusInput, RuleSuggestion, RuleSuggestionPage,
    RuleSuggestionQuery, RuleUpdateInput, RuleValidationMatch, RuleValidationPage,
    RuleValidationQuery, StoredContentItem, XDislikeQuery, XDislikedPost,
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::collections::BTreeMap;

pub(super) const DEFAULT_RULE_ID: &str = "x-engagement-bait-reaction";
pub(super) const DEFAULT_RULE_STATUS: &str = "active";
pub(super) const DEFAULT_RULE_PRIORITY: i64 = 50;
pub(super) const DEFAULT_RULE_TITLE: &str = "Engagement bait reaction posts";
pub(super) const DEFAULT_RULE_INSTRUCTION: &str = "Downrank engagement bait, dunking, or 'look at this absurd thing' posts where the main content is a reaction to a video, image, or quote rather than a substantive claim.";
pub(super) const DEFAULT_RULE_SOURCE: &str = "seed";
pub(super) const DEFAULT_NEW_RULE_STATUS: &str = "draft";
pub(super) const DEFAULT_NEW_RULE_PRIORITY: i64 = 100;
const RULE_AUDIT_LIMIT: usize = 50;
const CONTENT_RULE_COLUMNS: &str = concat!(
    "id, ",
    "site, ",
    "status, ",
    "priority, ",
    "title, ",
    "instruction, ",
    "created_source, ",
    "created_at_unix_ms, ",
    "updated_at_unix_ms, ",
    "examples_json"
);

impl Store {
    pub(in crate::storage) fn rules(&self, query: RuleQuery) -> Result<RulePage> {
        let status = clean_optional(query.status.as_deref());
        let limit = sqlite_limit(query.limit);
        let offset = sqlite_limit(query.offset);
        let total_matching = self.connection.query_row(
            "
            SELECT COUNT(*)
            FROM content_rules
            WHERE (?1 IS NULL OR status = ?1)
            ",
            [status.as_deref()],
            |row| row.get::<_, i64>(0),
        )?;

        let sql = format!(
            "
            SELECT {CONTENT_RULE_COLUMNS}
            FROM content_rules
            WHERE (?1 IS NULL OR status = ?1)
            ORDER BY priority ASC, id ASC
            LIMIT ?2 OFFSET ?3
            "
        );
        let mut statement = self.connection.prepare(&sql)?;
        let items = statement
            .query_map(
                params![status.as_deref(), limit, offset],
                content_rule_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(RulePage {
            total_matching: total_matching.max(0) as usize,
            limit: limit as usize,
            offset: offset as usize,
            items,
        })
    }

    pub(in crate::storage) fn rule_detail(&self, id: &str) -> Result<Option<RuleDetail>> {
        let Some(id) = clean_rule_id_for_lookup(id) else {
            return Ok(None);
        };
        let Some(rule) = self.rule(&id)? else {
            return Ok(None);
        };
        let audit = self.rule_audit(&id, RULE_AUDIT_LIMIT)?;

        Ok(Some(RuleDetail { rule, audit }))
    }

    pub(in crate::storage) fn create_rule(
        &mut self,
        input: RuleCreateInput,
    ) -> Result<ContentRule> {
        let now = now_unix_ms();
        let title = required_clean_text(&input.title, "title")?;
        let instruction = required_clean_text(&input.instruction, "instruction")?;
        let created_source = required_clean_text(&input.created_source, "source")?;
        let status = clean_rule_status(input.status.as_deref().unwrap_or(DEFAULT_NEW_RULE_STATUS))?;
        let priority = input.priority.unwrap_or(DEFAULT_NEW_RULE_PRIORITY);
        let examples = clean_examples(input.examples);
        let id = match input.id {
            Some(id) => clean_rule_id(&id)?,
            None => generated_rule_id(&title, &instruction, now),
        };

        if self.rule(&id)?.is_some() {
            return Err(StorageError::InvalidInput(format!(
                "rule id already exists: {id}"
            )));
        }

        let examples_json = serde_json::to_string(&examples)?;
        self.connection.execute(
            "
            INSERT INTO content_rules (
                id,
                site,
                status,
                priority,
                title,
                instruction,
                created_source,
                created_at_unix_ms,
                updated_at_unix_ms,
                examples_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)
            ",
            params![
                id,
                SITE_DIR,
                status,
                priority,
                title,
                instruction,
                created_source,
                now,
                examples_json,
            ],
        )?;

        let rule = self
            .rule(&id)?
            .expect("created rule should be readable immediately");
        self.record_rule_event(&rule, "created", &rule.created_source, now)?;

        Ok(rule)
    }

    pub(in crate::storage) fn update_rule(
        &mut self,
        input: RuleUpdateInput,
    ) -> Result<Option<ContentRule>> {
        let Some(id) = clean_rule_id_for_lookup(&input.id) else {
            return Ok(None);
        };
        let Some(existing) = self.rule(&id)? else {
            return Ok(None);
        };

        let source = required_clean_text(&input.source, "source")?;
        let status = match input.status {
            Some(status) => clean_rule_status(&status)?,
            None => existing.status,
        };
        let priority = input.priority.unwrap_or(existing.priority);
        let title = match input.title {
            Some(title) => required_clean_text(&title, "title")?,
            None => existing.title,
        };
        let instruction = match input.instruction {
            Some(instruction) => required_clean_text(&instruction, "instruction")?,
            None => existing.instruction,
        };
        let mut examples = existing.examples;
        if let Some(positive_examples) = input.positive_examples {
            examples.positive = clean_example_list(positive_examples);
        }
        if let Some(negative_examples) = input.negative_examples {
            examples.negative = clean_example_list(negative_examples);
        }
        let examples_json = serde_json::to_string(&examples)?;
        let now = now_unix_ms();

        self.connection.execute(
            "
            UPDATE content_rules
            SET
                status = ?2,
                priority = ?3,
                title = ?4,
                instruction = ?5,
                updated_at_unix_ms = ?6,
                examples_json = ?7
            WHERE id = ?1
            ",
            params![id, status, priority, title, instruction, now, examples_json],
        )?;

        let rule = self
            .rule(&id)?
            .expect("updated rule should be readable immediately");
        self.record_rule_event(&rule, "updated", &source, now)?;

        Ok(Some(rule))
    }

    pub(in crate::storage) fn update_rule_status(
        &mut self,
        input: RuleStatusInput,
    ) -> Result<Option<ContentRule>> {
        let Some(id) = clean_rule_id_for_lookup(&input.id) else {
            return Ok(None);
        };
        if self.rule(&id)?.is_none() {
            return Ok(None);
        }

        let status = clean_rule_status(&input.status)?;
        let source = required_clean_text(&input.source, "source")?;
        let now = now_unix_ms();
        self.connection.execute(
            "
            UPDATE content_rules
            SET status = ?2, updated_at_unix_ms = ?3
            WHERE id = ?1
            ",
            params![id, status, now],
        )?;

        let rule = self
            .rule(&id)?
            .expect("status-updated rule should be readable immediately");
        self.record_rule_event(&rule, rule_status_event_kind(&rule.status), &source, now)?;

        Ok(Some(rule))
    }

    pub(in crate::storage) fn validate_rule(
        &self,
        id: &str,
        query: RuleValidationQuery,
    ) -> Result<Option<RuleValidationPage>> {
        let Some(id) = clean_rule_id_for_lookup(id) else {
            return Ok(None);
        };
        let Some(rule) = self.rule(&id)? else {
            return Ok(None);
        };

        let matcher = RuleMatcher::new(&rule);
        let total_stored = self
            .connection
            .query_row("SELECT COUNT(*) FROM tweets", [], |row| {
                row.get::<_, i64>(0)
            })?;
        let mut statement = self.connection.prepare(
            "
            SELECT
                storage_key,
                post_id,
                url,
                author_handle,
                text,
                first_seen_at_unix_ms,
                last_seen_at_unix_ms,
                seen_count,
                latest_captured_at
            FROM tweets
            ORDER BY last_seen_at_unix_ms DESC, storage_key ASC
            ",
        )?;
        let mut matches = Vec::new();
        let rows = statement.query_map([], |row| content_item_from_row(row, None))?;
        for row in rows {
            let content = row?;
            if let Some(rule_match) = matcher.evaluate(content) {
                matches.push(rule_match);
            }
        }

        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| {
                    right
                        .content
                        .last_seen_at_unix_ms
                        .cmp(&left.content.last_seen_at_unix_ms)
                })
                .then_with(|| left.content.storage_key.cmp(&right.content.storage_key))
        });

        let limit = query.limit.min(i64::MAX as usize);
        let offset = query.offset.min(i64::MAX as usize);
        let total_matching = matches.len();
        let items = matches.into_iter().skip(offset).take(limit).collect();

        Ok(Some(RuleValidationPage {
            rule,
            total_stored: total_stored.max(0) as usize,
            total_matching,
            limit,
            offset,
            items,
        }))
    }

    pub(in crate::storage) fn rule_suggestions(
        &self,
        query: RuleSuggestionQuery,
    ) -> Result<RuleSuggestionPage> {
        let min_feedback = query.min_feedback.max(1);
        let limit = query.limit.min(i64::MAX as usize);
        let offset = query.offset.min(i64::MAX as usize);
        let page = self.dislikes(XDislikeQuery {
            active: Some(true),
            unprocessed: None,
            limit: 500,
            offset: 0,
        })?;
        let mut groups = BTreeMap::<String, Vec<XDislikedPost>>::new();

        for item in page.items {
            let reason = clean_feedback_reason(&item.reason);
            groups.entry(reason).or_default().push(item);
        }

        let mut suggestions = groups
            .into_iter()
            .filter_map(|(reason, evidence)| {
                (evidence.len() >= min_feedback)
                    .then(|| rule_suggestion_from_feedback(reason, evidence))
            })
            .collect::<Vec<_>>();
        suggestions.sort_by(|left, right| {
            right
                .feedback_count
                .cmp(&left.feedback_count)
                .then_with(|| left.title.cmp(&right.title))
        });

        let total_matching = suggestions.len();
        let items = suggestions.into_iter().skip(offset).take(limit).collect();

        Ok(RuleSuggestionPage {
            total_matching,
            limit,
            offset,
            items,
        })
    }

    pub(in crate::storage) fn create_rule_set_proposal(
        &mut self,
        input: RuleSetProposalCreateInput,
    ) -> Result<RuleSetProposal> {
        let now = now_unix_ms();
        let source = required_clean_text(&input.source, "source")?;
        let changes = clean_rule_set_proposal_changes(input.changes)?;
        let changes_json = serde_json::to_string(&changes)?;
        let id = generated_rule_set_proposal_id(
            &source,
            input.feedback_count,
            input.active_rule_count,
            &changes_json,
            now,
        );
        let proposal = RuleSetProposal {
            id,
            site: SITE_DIR.into(),
            status: "pending".into(),
            source,
            created_at_unix_ms: now,
            feedback_count: input.feedback_count,
            active_rule_count: input.active_rule_count,
            changes,
        };
        let proposal_json = serde_json::to_string(&proposal)?;

        self.connection.execute(
            "
            INSERT INTO rule_set_proposals (
                id,
                site,
                status,
                source,
                created_at_unix_ms,
                proposal_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ",
            params![
                proposal.id.as_str(),
                proposal.site.as_str(),
                proposal.status.as_str(),
                proposal.source.as_str(),
                proposal.created_at_unix_ms,
                proposal_json,
            ],
        )?;

        Ok(proposal)
    }

    pub(in crate::storage) fn rule_set_proposals(
        &self,
        query: RuleSetProposalQuery,
    ) -> Result<RuleSetProposalPage> {
        let status = query
            .status
            .as_deref()
            .map(clean_rule_set_proposal_status)
            .transpose()?;
        let limit = sqlite_limit(query.limit);
        let offset = sqlite_limit(query.offset);
        let total_matching = self.connection.query_row(
            "
            SELECT COUNT(*)
            FROM rule_set_proposals
            WHERE (?1 IS NULL OR status = ?1)
            ",
            [status.as_deref()],
            |row| row.get::<_, i64>(0),
        )?;

        let mut statement = self.connection.prepare(
            "
            SELECT
                id,
                site,
                status,
                source,
                created_at_unix_ms,
                proposal_json
            FROM rule_set_proposals
            WHERE (?1 IS NULL OR status = ?1)
            ORDER BY created_at_unix_ms DESC, id DESC
            LIMIT ?2 OFFSET ?3
            ",
        )?;
        let items = statement
            .query_map(params![status.as_deref(), limit, offset], |row| {
                rule_set_proposal_from_row(row)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(RuleSetProposalPage {
            total_matching: total_matching.max(0) as usize,
            limit: limit as usize,
            offset: offset as usize,
            items,
        })
    }

    pub(in crate::storage) fn rule_set_proposal(
        &self,
        id: &str,
    ) -> Result<Option<RuleSetProposal>> {
        let Some(id) = clean_rule_id_for_lookup(id) else {
            return Ok(None);
        };

        load_rule_set_proposal(&self.connection, &id)
    }

    fn rule(&self, id: &str) -> Result<Option<ContentRule>> {
        load_rule(&self.connection, id)
    }

    fn rule_audit(&self, id: &str, limit: usize) -> Result<Vec<RuleAuditEvent>> {
        let limit = sqlite_limit(limit);
        let mut statement = self.connection.prepare(
            "
            SELECT
                id,
                rule_id,
                event_kind,
                source,
                created_at_unix_ms,
                snapshot_json
            FROM content_rule_events
            WHERE rule_id = ?1
            ORDER BY created_at_unix_ms DESC, id DESC
            LIMIT ?2
            ",
        )?;
        let events = statement
            .query_map(params![id, limit], rule_audit_event_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(events)
    }

    fn record_rule_event(
        &self,
        rule: &ContentRule,
        event_kind: &str,
        source: &str,
        created_at_unix_ms: i64,
    ) -> Result<()> {
        insert_rule_event(
            &self.connection,
            rule,
            event_kind,
            source,
            created_at_unix_ms,
        )
    }
}

pub(super) fn seed_default_rules(connection: &Connection) -> Result<()> {
    let now = now_unix_ms();
    let examples_json = serde_json::to_string(&RuleExamples {
        positive: Vec::new(),
        negative: Vec::new(),
    })?;

    let inserted = connection.execute(
        "
        INSERT OR IGNORE INTO content_rules (
            id,
            site,
            status,
            priority,
            title,
            instruction,
            created_source,
            created_at_unix_ms,
            updated_at_unix_ms,
            examples_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)
        ",
        params![
            DEFAULT_RULE_ID,
            SITE_DIR,
            DEFAULT_RULE_STATUS,
            DEFAULT_RULE_PRIORITY,
            DEFAULT_RULE_TITLE,
            DEFAULT_RULE_INSTRUCTION,
            DEFAULT_RULE_SOURCE,
            now,
            examples_json,
        ],
    )?;
    if rule_event_count(connection, DEFAULT_RULE_ID)? == 0 {
        let rule = load_rule(connection, DEFAULT_RULE_ID)?
            .expect("default rule should exist after seed insert");
        let event_kind = if inserted > 0 { "created" } else { "imported" };
        insert_rule_event(connection, &rule, event_kind, &rule.created_source, now)?;
    }

    Ok(())
}
fn content_rule_from_row(row: &Row<'_>) -> rusqlite::Result<ContentRule> {
    let examples_json: String = row.get(9)?;
    let examples = serde_json::from_str(&examples_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(error))
    })?;

    Ok(ContentRule {
        id: row.get(0)?,
        site: row.get(1)?,
        status: row.get(2)?,
        priority: row.get(3)?,
        title: row.get(4)?,
        instruction: row.get(5)?,
        created_source: row.get(6)?,
        created_at_unix_ms: row.get(7)?,
        updated_at_unix_ms: row.get(8)?,
        examples,
    })
}

fn load_rule(connection: &Connection, id: &str) -> Result<Option<ContentRule>> {
    let sql = format!(
        "
        SELECT {CONTENT_RULE_COLUMNS}
        FROM content_rules
        WHERE id = ?1
        "
    );
    Ok(connection
        .query_row(&sql, [id], content_rule_from_row)
        .optional()?)
}

fn rule_event_count(connection: &Connection, rule_id: &str) -> Result<i64> {
    Ok(connection.query_row(
        "
        SELECT COUNT(*)
        FROM content_rule_events
        WHERE rule_id = ?1
        ",
        [rule_id],
        |row| row.get::<_, i64>(0),
    )?)
}

fn insert_rule_event(
    connection: &Connection,
    rule: &ContentRule,
    event_kind: &str,
    source: &str,
    created_at_unix_ms: i64,
) -> Result<()> {
    let snapshot_json = serde_json::to_string(rule)?;
    connection.execute(
        "
        INSERT INTO content_rule_events (
            rule_id,
            event_kind,
            source,
            created_at_unix_ms,
            snapshot_json
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        ",
        params![
            rule.id.as_str(),
            event_kind,
            source,
            created_at_unix_ms,
            snapshot_json
        ],
    )?;

    Ok(())
}

fn rule_audit_event_from_row(row: &Row<'_>) -> rusqlite::Result<RuleAuditEvent> {
    let snapshot_json: String = row.get(5)?;
    let snapshot = serde_json::from_str(&snapshot_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;

    Ok(RuleAuditEvent {
        id: row.get(0)?,
        rule_id: row.get(1)?,
        event_kind: row.get(2)?,
        source: row.get(3)?,
        created_at_unix_ms: row.get(4)?,
        snapshot,
    })
}

fn rule_set_proposal_from_row(row: &Row<'_>) -> rusqlite::Result<RuleSetProposal> {
    let proposal_json: String = row.get(5)?;
    let mut proposal: RuleSetProposal = serde_json::from_str(&proposal_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;

    proposal.id = row.get(0)?;
    proposal.site = row.get(1)?;
    proposal.status = row.get(2)?;
    proposal.source = row.get(3)?;
    proposal.created_at_unix_ms = row.get(4)?;

    Ok(proposal)
}

fn load_rule_set_proposal(connection: &Connection, id: &str) -> Result<Option<RuleSetProposal>> {
    Ok(connection
        .query_row(
            "
            SELECT
                id,
                site,
                status,
                source,
                created_at_unix_ms,
                proposal_json
            FROM rule_set_proposals
            WHERE id = ?1
            ",
            [id],
            rule_set_proposal_from_row,
        )
        .optional()?)
}

fn required_clean_text(value: &str, field: &str) -> Result<String> {
    clean_optional(Some(value))
        .ok_or_else(|| StorageError::InvalidInput(format!("{field} must not be empty")))
}

fn clean_rule_id(value: &str) -> Result<String> {
    let id = required_clean_text(value, "id")?;
    let valid = id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_');
    if !valid {
        return Err(StorageError::InvalidInput(
            "id may contain only ASCII letters, numbers, hyphens, and underscores".into(),
        ));
    }

    Ok(id)
}

fn clean_rule_id_for_lookup(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn clean_rule_status(value: &str) -> Result<String> {
    let status = required_clean_text(value, "status")?.to_ascii_lowercase();
    match status.as_str() {
        "draft" | "active" | "disabled" | "archived" => Ok(status),
        _ => Err(StorageError::InvalidInput(format!(
            "unsupported rule status: {status}"
        ))),
    }
}

fn clean_rule_set_proposal_status(value: &str) -> Result<String> {
    let status = required_clean_text(value, "status")?.to_ascii_lowercase();
    match status.as_str() {
        "pending" | "applied" | "dismissed" => Ok(status),
        _ => Err(StorageError::InvalidInput(format!(
            "unsupported rule-set proposal status: {status}"
        ))),
    }
}

fn clean_examples(examples: RuleExamples) -> RuleExamples {
    RuleExamples {
        positive: clean_example_list(examples.positive),
        negative: clean_example_list(examples.negative),
    }
}

fn clean_example_list(examples: Vec<String>) -> Vec<String> {
    let mut cleaned = Vec::new();
    for example in examples {
        let example = normalize_text(&example);
        if example.is_empty() || cleaned.contains(&example) {
            continue;
        }
        cleaned.push(example);
    }

    cleaned
}

fn clean_rule_set_proposal_changes(
    changes: Vec<RuleSetProposalChange>,
) -> Result<Vec<RuleSetProposalChange>> {
    let mut cleaned = Vec::new();
    for change in changes {
        cleaned.push(clean_rule_set_proposal_change(change)?);
    }

    if cleaned.is_empty() {
        cleaned.push(RuleSetProposalChange {
            action: RuleSetProposalAction::NoChange,
            rule_id: None,
            status: None,
            priority: None,
            title: None,
            instruction: None,
            rationale: "No rule-set change is currently justified.".into(),
            evidence_storage_keys: Vec::new(),
            examples: RuleExamples::default(),
        });
    }

    Ok(cleaned)
}

fn clean_rule_set_proposal_change(change: RuleSetProposalChange) -> Result<RuleSetProposalChange> {
    let rule_id = change.rule_id.as_deref().map(clean_rule_id).transpose()?;
    let status = change
        .status
        .as_deref()
        .map(clean_rule_status)
        .transpose()?;
    let title = match change.title {
        Some(title) => Some(required_clean_text(&title, "title")?),
        None => None,
    };
    let instruction = match change.instruction {
        Some(instruction) => Some(required_clean_text(&instruction, "instruction")?),
        None => None,
    };
    let rationale = clean_optional(Some(&change.rationale))
        .unwrap_or_else(|| "Rule-set proposal generated from feedback.".into());
    let mut evidence_storage_keys = Vec::new();
    for key in change.evidence_storage_keys {
        let Some(key) = clean_optional(Some(&key)) else {
            continue;
        };
        if !evidence_storage_keys.contains(&key) {
            evidence_storage_keys.push(key);
        }
    }
    let examples = clean_examples(change.examples);

    match change.action {
        RuleSetProposalAction::CreateRule => {
            if title.is_none() {
                return Err(StorageError::InvalidInput(
                    "createRule proposals require title".into(),
                ));
            }
            if instruction.is_none() {
                return Err(StorageError::InvalidInput(
                    "createRule proposals require instruction".into(),
                ));
            }
        }
        RuleSetProposalAction::UpdateRule | RuleSetProposalAction::DisableRule => {
            if rule_id.is_none() {
                return Err(StorageError::InvalidInput(format!(
                    "{:?} proposals require ruleId",
                    change.action
                )));
            }
        }
        RuleSetProposalAction::NoChange => {}
    }

    Ok(RuleSetProposalChange {
        action: change.action,
        rule_id,
        status,
        priority: change.priority,
        title,
        instruction,
        rationale,
        evidence_storage_keys,
        examples,
    })
}

fn generated_rule_id(title: &str, instruction: &str, now_unix_ms: i64) -> String {
    let slug = rule_slug(title);
    let hash = stable_hash(&format!("{title}\n{instruction}\n{now_unix_ms}"));
    format!("x-{slug}-{:08x}", hash as u32)
}

fn generated_rule_set_proposal_id(
    source: &str,
    feedback_count: usize,
    active_rule_count: usize,
    changes_json: &str,
    now_unix_ms: i64,
) -> String {
    let hash = stable_hash(&format!(
        "{source}\n{feedback_count}\n{active_rule_count}\n{changes_json}\n{now_unix_ms}"
    ));
    format!("x-rule-proposal-{now_unix_ms}-{hash:08x}")
}

fn rule_slug(title: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;

    for character in title.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_was_separator = false;
        } else if !last_was_separator && !slug.is_empty() {
            slug.push('-');
            last_was_separator = true;
        }

        if slug.len() >= 48 {
            break;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "rule".into()
    } else {
        slug
    }
}

fn rule_status_event_kind(status: &str) -> &'static str {
    match status {
        "active" => "enabled",
        "disabled" => "disabled",
        "archived" => "archived",
        "draft" => "drafted",
        _ => "statusChanged",
    }
}

struct RuleMatcher {
    terms: Vec<String>,
    positive_examples: Vec<String>,
    negative_examples: Vec<String>,
}

impl RuleMatcher {
    fn new(rule: &ContentRule) -> Self {
        let mut terms = Vec::new();
        for source in [&rule.title, &rule.instruction] {
            collect_match_terms(source, &mut terms);
        }
        for example in &rule.examples.positive {
            collect_match_terms(example, &mut terms);
        }

        Self {
            terms,
            positive_examples: normalized_examples(&rule.examples.positive),
            negative_examples: normalized_examples(&rule.examples.negative),
        }
    }

    fn evaluate(&self, content: StoredContentItem) -> Option<RuleValidationMatch> {
        let text = content.text.to_ascii_lowercase();
        if text.is_empty()
            || self
                .negative_examples
                .iter()
                .any(|example| text.contains(example))
        {
            return None;
        }

        let matched_terms = self
            .terms
            .iter()
            .filter(|term| text_contains_token(&text, term))
            .cloned()
            .collect::<Vec<_>>();
        let matched_examples = self
            .positive_examples
            .iter()
            .filter(|example| text.contains(*example))
            .cloned()
            .collect::<Vec<_>>();

        if matched_terms.is_empty() && matched_examples.is_empty() {
            return None;
        }

        let score = matched_terms.len() + matched_examples.len() * 5;
        Some(RuleValidationMatch {
            content,
            score,
            matched_terms,
            matched_examples,
        })
    }
}

fn collect_match_terms(text: &str, terms: &mut Vec<String>) {
    for token in text
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 4)
        .map(str::to_ascii_lowercase)
    {
        if is_rule_match_stop_word(&token) || terms.contains(&token) {
            continue;
        }
        terms.push(token);
    }
}

fn normalized_examples(examples: &[String]) -> Vec<String> {
    examples
        .iter()
        .map(|example| normalize_text(example).to_ascii_lowercase())
        .filter(|example| example.len() >= 8)
        .collect()
}

fn text_contains_token(text: &str, token: &str) -> bool {
    text.split(|character: char| !character.is_ascii_alphanumeric())
        .any(|candidate| candidate.eq_ignore_ascii_case(token))
}

fn is_rule_match_stop_word(token: &str) -> bool {
    matches!(
        token,
        "about"
            | "active"
            | "after"
            | "again"
            | "against"
            | "because"
            | "content"
            | "downrank"
            | "from"
            | "hide"
            | "like"
            | "main"
            | "more"
            | "post"
            | "posts"
            | "rather"
            | "rule"
            | "rules"
            | "should"
            | "site"
            | "source"
            | "status"
            | "than"
            | "that"
            | "this"
            | "tweet"
            | "tweets"
            | "where"
            | "with"
    )
}

fn clean_feedback_reason(reason: &str) -> String {
    let reason = normalize_text(reason);
    if reason.is_empty() {
        "thumbs-down feedback".into()
    } else {
        reason
    }
}

fn rule_suggestion_from_feedback(
    reason: String,
    mut evidence: Vec<XDislikedPost>,
) -> RuleSuggestion {
    evidence.sort_by(|left, right| {
        right
            .updated_at_unix_ms
            .cmp(&left.updated_at_unix_ms)
            .then_with(|| left.storage_key.cmp(&right.storage_key))
    });
    let title_reason = sentence_case(&reason);
    let title = format!("Feedback: {}", truncate_for_rule(&title_reason, 72));
    let instruction = format!(
        "Hide X posts similar to posts the user disliked for this reason: {}.",
        reason
    );
    let examples = RuleExamples {
        positive: evidence
            .iter()
            .filter_map(|item| clean_optional(Some(item.text.as_str())))
            .take(5)
            .collect(),
        negative: Vec::new(),
    };
    let id = format!(
        "x-feedback-{}-{:08x}",
        rule_slug(&reason),
        stable_hash(&reason) as u32
    );

    RuleSuggestion {
        id,
        status: DEFAULT_NEW_RULE_STATUS.into(),
        priority: DEFAULT_NEW_RULE_PRIORITY,
        title,
        instruction,
        source: "feedback".into(),
        feedback_count: evidence.len(),
        reasons: vec![reason],
        examples,
        evidence: evidence.into_iter().take(10).collect(),
    }
}

fn sentence_case(text: &str) -> String {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };

    format!("{}{}", first.to_uppercase(), chars.collect::<String>())
}

fn truncate_for_rule(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
