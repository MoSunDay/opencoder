//! Team topic-run persistence (`team_topic_runs`) — the (topic, node)
//! fan-out ledger of the opencode-team runtime.
//!
//! Free functions over a raw `Connection`, mirroring sibling submodules
//! (`brain.rs` / `project_runs.rs`). The DDL constants live here (not in
//! `schema.rs`) so the domain owns its tables; `schema.rs` imports and
//! registers them in the bootstrap batch + v17 migration.

use anyhow::{Context, Result};
use libsql::{params, Connection, Row};

use crate::team_types::TeamTopicRunRecord;

/// Table DDL registered by `schema.rs` (bootstrap batch + v17 migration).
/// One row per (topic, node) pairing: `status` starts `executing` and flips
/// to `finished`; `created_at` is stamped at first insert and never moves.
/// Cascades with the node so a deregistered worker leaves no orphan pairings.
pub(super) const CREATE_TEAM_TOPIC_RUNS: &str = "\
CREATE TABLE IF NOT EXISTS team_topic_runs (
  topic_id TEXT NOT NULL,
  node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  status TEXT NOT NULL DEFAULT 'executing',
  created_at INTEGER NOT NULL,
  PRIMARY KEY (topic_id, node_id)
)";
/// Per-topic listings are ordered scans over the topic's rows; the index
/// keeps them off full table scans (the PK already covers it on fresh
/// SQLite builds, but the explicit index pins the access path).
pub(super) const CREATE_INDEX_TEAM_TOPIC_RUNS_TOPIC: &str =
    "CREATE INDEX IF NOT EXISTS idx_team_topic_runs_topic ON team_topic_runs(topic_id)";

const RUN_COLS: &str = "topic_id, node_id, status, created_at";

/// Insert or refresh one `(topic_id, node_id)` run row. INSERT OR REPLACE
/// semantics with one carve-out: an existing row keeps its original
/// `created_at` — the conflict arm updates ONLY `status`, so a refresh
/// (e.g. re-announcing an executing pairing) never restarts the run's clock.
pub async fn upsert(conn: &Connection, rec: &TeamTopicRunRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO team_topic_runs (topic_id, node_id, status, created_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(topic_id, node_id) DO UPDATE SET status = excluded.status",
        params![
            rec.topic_id.as_str(),
            rec.node_id.as_str(),
            rec.status.as_str(),
            rec.created_at
        ],
    )
    .await
    .context("upsert team topic run")?;
    Ok(())
}

/// Flip EVERY row of `topic_id` to `finished`. No-op (0 rows) for unknown or
/// already-finished topics — callers treat "nothing to finish" as success.
pub async fn finish(conn: &Connection, topic_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE team_topic_runs SET status = 'finished' WHERE topic_id = ?1",
        params![topic_id],
    )
    .await
    .context("finish team topic runs")?;
    Ok(())
}

/// All run rows of `topic_id`, oldest `created_at` first (rowid breaks
/// same-ms ties; ULIDs are not monotonic and must never order anything).
pub async fn list(conn: &Connection, topic_id: &str) -> Result<Vec<TeamTopicRunRecord>> {
    let stmt = conn
        .prepare(&format!(
            "SELECT {RUN_COLS} FROM team_topic_runs
             WHERE topic_id = ?1 ORDER BY created_at ASC, rowid ASC"
        ))
        .await?;
    let mut rows = stmt.query(params![topic_id]).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_record(&r)?);
    }
    Ok(out)
}

fn row_to_record(r: &Row) -> Result<TeamTopicRunRecord> {
    Ok(TeamTopicRunRecord {
        topic_id: r.get(0)?,
        node_id: r.get(1)?,
        status: r.get(2)?,
        created_at: r.get(3)?,
    })
}
