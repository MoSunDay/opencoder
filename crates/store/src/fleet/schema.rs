use anyhow::{bail, Context, Result};
use libsql::{params, Connection};
use opencoder_core::fleet::{valid_id, ExecutionKind, ExecutionStatus};
use serde_json::Value;

const CREATE_NODES: &str =
    "CREATE TABLE IF NOT EXISTS fleet_nodes (id TEXT PRIMARY KEY, registration TEXT NOT NULL)";
const CREATE_DEFINITIONS: &str = "CREATE TABLE IF NOT EXISTS fleet_definitions (kind TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY(kind,id))";
const CREATE_INDEX: &str = "CREATE TABLE execution_index (id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, kind TEXT NOT NULL, node_id TEXT NOT NULL, status TEXT NOT NULL)";
const CREATE_LOOKUP_INDEX: &str = "CREATE INDEX IF NOT EXISTS execution_node_kind_v3 ON execution_index(node_id, kind, created_at)";
const LEGACY_BACKUP: &str = "execution_index_v2_backup";

#[derive(Debug, PartialEq, Eq)]
struct Column {
    name: String,
    data_type: String,
    not_null: bool,
    default_value: Option<String>,
    primary_key: bool,
}

const LEGACY_COLUMNS: &[(&str, &str, bool, bool)] = &[
    ("id", "TEXT", false, true),
    ("created_at", "INTEGER", true, false),
    ("node_id", "TEXT", true, false),
    ("status", "TEXT", true, false),
];
const CURRENT_COLUMNS: &[(&str, &str, bool, bool)] = &[
    ("id", "TEXT", false, true),
    ("created_at", "INTEGER", true, false),
    ("kind", "TEXT", true, false),
    ("node_id", "TEXT", true, false),
    ("status", "TEXT", true, false),
];

pub(super) async fn initialize(conn: &Connection) -> Result<()> {
    conn.execute("BEGIN IMMEDIATE", ())
        .await
        .context("begin fleet schema transaction")?;
    match initialize_tx(conn).await {
        Ok(()) => {
            conn.execute("COMMIT", ())
                .await
                .context("commit fleet schema transaction")?;
            Ok(())
        }
        Err(error) => {
            if let Err(rollback) = conn.execute("ROLLBACK", ()).await {
                tracing::warn!(%rollback, "fleet schema rollback failed");
            }
            Err(error)
        }
    }
}

async fn initialize_tx(conn: &Connection) -> Result<()> {
    conn.execute(CREATE_NODES, ()).await?;
    conn.execute(CREATE_DEFINITIONS, ()).await?;
    match columns(conn, "execution_index").await? {
        None => {
            conn.execute(CREATE_INDEX, ()).await?;
        }
        Some(columns) if exact_schema(&columns, CURRENT_COLUMNS) => {
            validate_current_rows(conn).await?;
        }
        Some(columns) if exact_schema(&columns, LEGACY_COLUMNS) => {
            migrate_legacy_index(conn).await?;
        }
        Some(columns) => bail!(
            "unsupported execution_index schema; expected exact v2 or v3 columns, found {columns:?}"
        ),
    }
    conn.execute(CREATE_LOOKUP_INDEX, ()).await?;
    Ok(())
}

async fn migrate_legacy_index(conn: &Connection) -> Result<()> {
    if columns(conn, LEGACY_BACKUP).await?.is_some() {
        bail!("legacy execution index backup already exists; refusing ambiguous migration");
    }
    let mut rows = conn
        .query(
            "SELECT id,created_at,node_id,status FROM execution_index ORDER BY id",
            (),
        )
        .await?;
    let mut records = Vec::new();
    while let Some(row) = rows.next().await? {
        let id: String = row.get(0)?;
        if !valid_id(&id) {
            bail!("legacy execution has invalid id: {id}");
        }
        let kind = legacy_kind(&id)
            .with_context(|| format!("legacy execution {id} has no recognized kind prefix"))?;
        let status: String = row.get(3)?;
        serde_json::from_value::<ExecutionStatus>(Value::String(status.clone()))
            .with_context(|| format!("legacy execution {id} has invalid status"))?;
        records.push((id, row.get::<i64>(1)?, kind, row.get::<String>(2)?, status));
    }
    drop(rows);
    conn.execute(
        &format!("ALTER TABLE execution_index RENAME TO {LEGACY_BACKUP}"),
        (),
    )
    .await?;
    conn.execute(CREATE_INDEX, ()).await?;
    for (id, created_at, kind, node_id, status) in records {
        conn.execute(
            "INSERT INTO execution_index(id,created_at,kind,node_id,status) VALUES (?1,?2,?3,?4,?5)",
            params![id, created_at, kind, node_id, status],
        )
        .await?;
    }
    Ok(())
}

async fn validate_current_rows(conn: &Connection) -> Result<()> {
    let mut rows = conn
        .query("SELECT id,kind,status FROM execution_index", ())
        .await?;
    while let Some(row) = rows.next().await? {
        let id: String = row.get(0)?;
        if !valid_id(&id) {
            bail!("execution index has invalid id: {id}");
        }
        serde_json::from_value::<ExecutionKind>(Value::String(row.get(1)?))
            .with_context(|| format!("execution {id} has invalid kind"))?;
        serde_json::from_value::<ExecutionStatus>(Value::String(row.get(2)?))
            .with_context(|| format!("execution {id} has invalid status"))?;
    }
    Ok(())
}

fn legacy_kind(id: &str) -> Option<&'static str> {
    [
        "maintenance",
        "project",
        "system",
        "agent",
        "todos",
        "team",
        "dag",
    ]
    .into_iter()
    .find(|kind| id.starts_with(&format!("{kind}-")))
    .or_else(|| id.starts_with("prun-").then_some("project"))
}

fn exact_schema(columns: &[Column], expected: &[(&str, &str, bool, bool)]) -> bool {
    columns.len() == expected.len()
        && columns.iter().zip(expected).all(|(column, expected)| {
            column.name == expected.0
                && column.data_type.eq_ignore_ascii_case(expected.1)
                && column.not_null == expected.2
                && column.default_value.is_none()
                && column.primary_key == expected.3
        })
}

async fn columns(conn: &Connection, table: &str) -> Result<Option<Vec<Column>>> {
    let mut exists = conn
        .query(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
        )
        .await?;
    if exists.next().await?.is_none() {
        return Ok(None);
    }
    let mut rows = conn
        .query(&format!("PRAGMA table_info({table})"), ())
        .await?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next().await? {
        columns.push(Column {
            name: row.get(1)?,
            data_type: row.get(2)?,
            not_null: row.get::<i64>(3)? != 0,
            default_value: row.get(4)?,
            primary_key: row.get::<i64>(5)? != 0,
        });
    }
    Ok(Some(columns))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_prefixes_are_explicit_and_unknowns_fail_closed() {
        assert_eq!(legacy_kind("agent-a"), Some("agent"));
        assert_eq!(legacy_kind("prun-a"), Some("project"));
        assert_eq!(legacy_kind("random-a"), None);
        assert_eq!(legacy_kind("agent"), None);
    }
}
