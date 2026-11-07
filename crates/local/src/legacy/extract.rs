//! Read-only extraction from a legacy SQLite session store.
//!
//! The source file is opened through the database crate's read-only
//! maintenance reader (`PRAGMA query_only`), so extraction can never mutate
//! application rows — only the explicit migration steps (backup, import,
//! marker) write anything, and only outside the source. One `SELECT` per
//! table pins the column order the strict parsers decode against; only the
//! `messages` rows with `summary = 0` are read (compaction summary rows are
//! rebuild artifacts, not transcript rows).
//!
//! Real fleets carry several legacy schema generations: columns were only
//! ever appended over time (`kind`, `harness_runtime`, `requirement`,
//! `provider_state_json`, `display`, `sse_kind`, ...) and whole tables
//! arrived late (`subagent_tasks`). Each SELECT is therefore projected
//! against the source's actual `pragma_table_info`: present columns are read
//! by name, absent ones are filled with `NULL AS <col>` (every late column
//! was nullable), and missing tables contribute zero rows. Values stay
//! strictly decoded — tolerating older layouts never means coercing cells.

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

/// Per-table extraction plan: projection order pinned to the parser layout,
/// optional row filter, deterministic output order.
struct TableSpec {
    table: &'static str,
    columns: &'static [&'static str],
    filter: &'static str,
    order_by: &'static str,
}

const SESSIONS_SPEC: TableSpec = TableSpec {
    table: "sessions",
    columns: SESSION_COLUMNS,
    filter: "",
    order_by: "id ASC",
};
const MESSAGES_SPEC: TableSpec = TableSpec {
    table: "messages",
    columns: MESSAGE_COLUMNS,
    filter: "WHERE summary = 0",
    order_by: "session_id ASC, seq ASC",
};
const EVENTS_SPEC: TableSpec = TableSpec {
    table: "session_events",
    columns: EVENT_COLUMNS,
    filter: "",
    order_by: "session_id ASC, seq ASC",
};
const INPUTS_SPEC: TableSpec = TableSpec {
    table: "session_inputs",
    columns: INPUT_COLUMNS,
    filter: "",
    order_by: "session_id ASC, seq ASC",
};
const TASKS_SPEC: TableSpec = TableSpec {
    table: "subagent_tasks",
    columns: TASK_COLUMNS,
    filter: "",
    order_by: "parent_session_id ASC, seq ASC",
};

/// Table names the migration understands; anything else in the source is left
/// untouched.
const KNOWN_TABLES: [&str; 5] = [
    "sessions",
    "messages",
    "session_events",
    "session_inputs",
    "subagent_tasks",
];

/// Run one SQL statement against the read-only maintenance reader.
async fn raw_query(source: &Path, sql: &str) -> Result<Vec<Vec<serde_json::Value>>> {
    let result = maintenance::query(source.to_path_buf(), sql.to_string(), Vec::new())
        .await
        .with_context(|| format!("query legacy store {}", source.display()))?;
    serde_json::from_value(
        result
            .get("rows")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .context("decode legacy query rows")
}

/// Actual column names of `table` in the source, per `pragma_table_info`.
async fn column_set(source: &Path, table: &str) -> Result<BTreeSet<String>> {
    let rows = raw_query(
        source,
        &format!("SELECT name FROM pragma_table_info('{table}')"),
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.first().and_then(|c| c.as_str().map(String::from)))
        .collect())
}

/// Rows for `spec`: absent tables contribute zero rows, present ones are
/// projected against their actual columns.
async fn table_rows(
    source: &Path,
    tables: &BTreeSet<String>,
    spec: &TableSpec,
) -> Result<Vec<Vec<serde_json::Value>>> {
    if !tables.contains(spec.table) {
        return Ok(Vec::new());
    }
    let available = column_set(source, spec.table).await?;
    query_rows(source, spec, &available).await
}

/// Project `spec` against the columns the source actually has: present
/// columns are selected by name in parser order, absent ones become `NULL`.
/// The result therefore always carries the exact layout the parsers pin.
async fn query_rows(
    source: &Path,
    spec: &TableSpec,
    available: &BTreeSet<String>,
) -> Result<Vec<Vec<serde_json::Value>>> {
    let projection = spec
        .columns
        .iter()
        .map(|column| {
            if available.contains(*column) {
                format!("\"{column}\"")
            } else {
                format!("NULL AS \"{column}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!("SELECT {projection} FROM {}", spec.table);
    if !spec.filter.is_empty() {
        sql.push(' ');
        sql.push_str(spec.filter);
    }
    sql.push_str(" ORDER BY ");
    sql.push_str(spec.order_by);
    raw_query(source, &sql).await
}

async fn table_names(source: &Path) -> Result<BTreeSet<String>> {
    let rows = raw_query(
        source,
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
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
    for row in table_rows(source, &tables, &SESSIONS_SPEC).await? {
        let id = row
            .first()
            .and_then(|c| c.as_str())
            .context("sessions row with non-TEXT id")?
            .to_string();
        dataset.sessions.push(parse::parse_session(&row, &id)?);
    }
    for row in table_rows(source, &tables, &MESSAGES_SPEC).await? {
        dataset.messages.push(parse::parse_message(&row)?);
    }
    for row in table_rows(source, &tables, &EVENTS_SPEC).await? {
        dataset.events.push(parse::parse_event(&row)?);
    }
    for row in table_rows(source, &tables, &INPUTS_SPEC).await? {
        dataset.inputs.push(parse::parse_input(&row)?);
    }
    for row in table_rows(source, &tables, &TASKS_SPEC).await? {
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
