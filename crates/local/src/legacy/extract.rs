//! Read-only extraction from a legacy SQLite session store.
//!
//! The source file is opened through the database crate's read-only
//! maintenance reader (`PRAGMA query_only`), so extraction can never mutate
//! application rows — only the explicit migration steps (backup, import,
//! marker) write anything, and only outside the source. One `SELECT` per
//! table pins the column order the strict parsers decode against; only the
//! `messages` rows with `summary = 0` are read (compaction summary rows are
//! rebuild artifacts, not transcript rows).

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};

use opencoder_database::maintenance;

use super::parse;
use super::rows::LegacyDataset;

pub(crate) const SESSION_COLUMNS: &[&str] = &[
    "id",
    "kind",
    "title",
    "agent",
    "model",
    "autopilot_mode",
    "workdir_hash",
    "created_at",
    "updated_at",
    "summary",
    "summary_seq",
    "summary_images_json",
    "handoff_seq",
    "handoff_plan",
    "skill",
    "task_type",
    "requirement",
];
pub(crate) const MESSAGE_COLUMNS: &[&str] = &[
    "session_id",
    "id",
    "role",
    "agent",
    "model",
    "blocks_json",
    "provider_state_json",
    "usage_json",
    "created_at",
    "synthetic",
    "display",
];
pub(crate) const EVENT_COLUMNS: &[&str] = &["session_id", "type", "payload_json", "sse_kind", "ts"];
pub(crate) const INPUT_COLUMNS: &[&str] = &[
    "session_id",
    "id",
    "delivery",
    "prompt",
    "images_json",
    "display_text",
    "admitted_seq",
    "promoted_seq",
];
pub(crate) const TASK_COLUMNS: &[&str] = &[
    "task_id",
    "parent_session_id",
    "child_session_id",
    "parent_message_id",
    "agent",
    "prompt",
    "result",
    "status",
    "ok",
    "started_at",
    "completed_at",
];

const SELECT_SESSIONS: &str = "SELECT id, kind, title, agent, model, autopilot_mode, \
     workdir_hash, created_at, updated_at, summary, summary_seq, summary_images_json, \
     handoff_seq, handoff_plan, skill, task_type, requirement FROM sessions \
     ORDER BY created_at ASC, id ASC";
const SELECT_MESSAGES: &str = "SELECT session_id, id, role, agent, model, blocks_json, \
     provider_state_json, usage_json, created_at, synthetic, display FROM messages \
     WHERE summary = 0 ORDER BY session_id ASC, seq ASC";
const SELECT_EVENTS: &str = "SELECT session_id, type, payload_json, sse_kind, ts \
     FROM session_events ORDER BY session_id ASC, seq ASC";
const SELECT_INPUTS: &str = "SELECT session_id, id, delivery, prompt, images_json, \
     display_text, admitted_seq, promoted_seq FROM session_inputs \
     ORDER BY session_id ASC, seq ASC";
const SELECT_TASKS: &str = "SELECT task_id, parent_session_id, child_session_id, \
     parent_message_id, agent, prompt, result, status, ok, started_at, completed_at \
     FROM subagent_tasks ORDER BY parent_session_id ASC, seq ASC";

/// Table names the migration understands; anything else in the source is left
/// untouched.
const KNOWN_TABLES: [&str; 5] = [
    "sessions",
    "messages",
    "session_events",
    "session_inputs",
    "subagent_tasks",
];

async fn query_table(
    source: &Path,
    sql: &str,
    expected_columns: &[&str],
) -> Result<Vec<Vec<serde_json::Value>>> {
    let result = maintenance::query(source.to_path_buf(), sql.to_string(), Vec::new())
        .await
        .with_context(|| format!("query legacy store {}", source.display()))?;
    let columns: Vec<String> = serde_json::from_value(
        result
            .get("columns")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .context("legacy query returned no column list")?;
    parse::require_columns(&columns, expected_columns)?;
    serde_json::from_value(
        result
            .get("rows")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .context("decode legacy query rows")
}

async fn table_names(source: &Path) -> Result<BTreeSet<String>> {
    let rows = query_table(
        source,
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        &["name"],
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.first().and_then(|c| c.as_str().map(String::from)))
        .collect())
}

/// Read every migratable row from the source store. Tables missing from a
/// very old source contribute zero rows and are reported by name.
pub(crate) async fn extract_dataset(source: &Path) -> Result<LegacyDataset> {
    anyhow::ensure!(
        source.is_file(),
        "legacy session store not found: {}",
        source.display()
    );
    let tables = table_names(source).await?;
    anyhow::ensure!(
        tables.contains("sessions"),
        "legacy store {} has no sessions table: {}",
        source.display(),
        if tables.is_empty() {
            "not an opencoder session store".to_string()
        } else {
            tables.iter().cloned().collect::<Vec<_>>().join(", ")
        }
    );

    let mut dataset = LegacyDataset::default();
    for row in query_table(source, SELECT_SESSIONS, SESSION_COLUMNS).await? {
        let id = row
            .first()
            .and_then(|c| c.as_str())
            .context("sessions row with non-TEXT id")?
            .to_string();
        dataset.sessions.push(parse::parse_session(&row, &id)?);
    }
    for row in query_table(source, SELECT_MESSAGES, MESSAGE_COLUMNS).await? {
        dataset.messages.push(parse::parse_message(&row)?);
    }
    for row in query_table(source, SELECT_EVENTS, EVENT_COLUMNS).await? {
        dataset.events.push(parse::parse_event(&row)?);
    }
    for row in query_table(source, SELECT_INPUTS, INPUT_COLUMNS).await? {
        dataset.inputs.push(parse::parse_input(&row)?);
    }
    for row in query_table(source, SELECT_TASKS, TASK_COLUMNS).await? {
        dataset
            .subagent_tasks
            .push(parse::parse_subagent_task(&row)?);
    }
    dataset.absent_tables = KNOWN_TABLES
        .into_iter()
        .filter(|t| !tables.contains(*t))
        .map(String::from)
        .collect();
    Ok(dataset)
}
