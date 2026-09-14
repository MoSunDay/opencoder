//! Worker-node registry (`nodes`).
//!
//! Free functions over a raw `Connection`, mirroring sibling submodules;
//! multi-statement ops run via [`super::tx::run_tx`] (`BEGIN IMMEDIATE`).

use anyhow::{Context, Result};
use libsql::{params, Connection};

use crate::types::NodeRecord;

const NODE_COLS: &str =
    "id, name, version, workdir, first_seen, last_seen_at, last_status, last_task_id, last_addr";

/// Register (or re-register) a node by its user-facing `name`.
///
/// A new name gets a fresh ULID; a known name keeps its original `id`
/// (tasks dispatched to it keep their FK) while version/workdir/last_seen_at
/// refresh and `last_status` resets to `online`.
pub async fn register(
    conn: &Connection,
    name: &str,
    version: Option<&str>,
    workdir: Option<&str>,
    addr: Option<&str>,
    now_ms: i64,
) -> Result<NodeRecord> {
    let id = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT INTO nodes (id, name, version, workdir, first_seen, last_seen_at, last_status, last_task_id, last_addr)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, 'online', NULL, ?6)
         ON CONFLICT(name) DO UPDATE SET
           version = excluded.version,
           workdir = excluded.workdir,
           last_seen_at = excluded.last_seen_at,
           last_status = 'online',
           last_addr = excluded.last_addr",
        params![id.as_str(), name, version, workdir, now_ms, addr],
    )
    .await
    .context("upsert node")?;
    get_by_name(conn, name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("register_node: row for {name:?} vanished after upsert"))
}

pub async fn list(conn: &Connection) -> Result<Vec<NodeRecord>> {
    let stmt = conn
        .prepare(&format!(
            "SELECT {NODE_COLS} FROM nodes ORDER BY first_seen ASC"
        ))
        .await?;
    let mut rows = stmt.query(()).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_node(&r)?);
    }
    Ok(out)
}

/// `SELECT ... WHERE id = ?1` over the node registry.
async fn get_where_id(conn: &Connection, col: &str, key: &str) -> Result<Option<NodeRecord>> {
    // `col` is a code-controlled literal ("id" | "name"), never user input.
    debug_assert!(matches!(col, "id" | "name"));
    let mut rows = conn
        .query(
            &format!("SELECT {NODE_COLS} FROM nodes WHERE {col} = ?1"),
            params![key],
        )
        .await?;
    rows.next().await?.map(|r| row_to_node(&r)).transpose()
}

pub async fn get(conn: &Connection, id: &str) -> Result<Option<NodeRecord>> {
    get_where_id(conn, "id", id).await
}

async fn get_by_name(conn: &Connection, name: &str) -> Result<Option<NodeRecord>> {
    get_where_id(conn, "name", name).await
}

pub async fn delete(conn: &Connection, id: &str) -> Result<()> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        // Sessions first: the schema's `node_tasks.session_id REFERENCES
        // sessions ON DELETE CASCADE` tears down the queue rows with them.
        conn.execute(
            "DELETE FROM sessions WHERE id IN (SELECT session_id FROM node_tasks WHERE node_id = ?1)",
            params![id],
        )
        .await
        .context("delete node task sessions")?;
        conn.execute("DELETE FROM node_tasks WHERE node_id = ?1", params![id])
            .await
            .context("delete node tasks")?;
        conn.execute("DELETE FROM nodes WHERE id = ?1", params![id])
            .await
            .context("delete node")?;
        Ok(())
    })
    .await
}

/// Liveness touch + cancel-command poll in one transaction. Refreshes
/// `last_seen_at`, collapses non-`busy` status toward `idle`, and returns
/// this node's cancelling task ids as the cancel commands.
pub async fn heartbeat(conn: &Connection, id: &str, now_ms: i64) -> Result<Vec<String>> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let changed = conn
            .execute(
                "UPDATE nodes SET last_seen_at = ?2,
                   last_status = CASE WHEN last_status != 'busy' THEN 'idle' ELSE last_status END
                 WHERE id = ?1",
                params![id, now_ms],
            )
            .await
            .context("heartbeat node")?;
        if changed == 0 {
            anyhow::bail!("heartbeat_node: node {id} not found");
        }
        let stmt = conn
            .prepare(
                "SELECT id FROM node_tasks WHERE node_id = ?1 AND status = 'cancelling'
                 AND cancel_requested = 1 ORDER BY created_at ASC, id ASC",
            )
            .await?;
        let mut rows = stmt.query(params![id]).await?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().await? {
            out.push(r.get::<String>(0)?);
        }
        Ok(out)
    })
    .await
}

fn row_to_node(r: &libsql::Row) -> Result<NodeRecord> {
    Ok(NodeRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        version: r.get(2)?,
        workdir: r.get(3)?,
        first_seen: r.get(4)?,
        last_seen_at: r.get(5)?,
        last_status: r.get(6)?,
        last_task_id: r.get(7)?,
        last_addr: r.get(8)?,
    })
}
