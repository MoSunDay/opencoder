//! Schedule fire persistence (`schedule_runs`) — the audit ledger of the
//! control-plane cron scheduler.
//!
//! Free functions over a raw `Connection`, mirroring sibling submodules
//! (`team_runs.rs` / `brain.rs`). The DDL constants live here (not in
//! `schema.rs`) so the domain owns its table; `schema.rs` imports and
//! registers it in the bootstrap batch + v26 migration.

use anyhow::{Context, Result};
use libsql::{params, Connection, Row};

use crate::schedule_types::ScheduleRunRecord;

/// Table DDL registered by `schema.rs` (bootstrap batch + v26 migration).
/// One row per (schedule, tick): the PK makes re-firing the same
/// `scheduled_for_ms` idempotent (an error-retry overwrites in place).
/// Terminal execution state lives in the execution index, not here.
pub(super) const CREATE_SCHEDULE_RUNS: &str = "\
CREATE TABLE IF NOT EXISTS schedule_runs (
  schedule_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  target TEXT NOT NULL,
  scheduled_for_ms INTEGER NOT NULL,
  fired_at_ms INTEGER NOT NULL,
  execution_id TEXT,
  status TEXT NOT NULL,
  error TEXT,
  missed INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (schedule_id, scheduled_for_ms)
)";
/// Cross-schedule recency scans (ops dashboards / the ctl history listing
/// across every schedule); per-schedule queries are covered by the PK.
pub(super) const CREATE_INDEX_SCHEDULE_RUNS_FIRED: &str =
    "CREATE INDEX IF NOT EXISTS idx_schedule_runs_fired ON schedule_runs(fired_at_ms)";

const RUN_COLS: &str =
    "schedule_id, kind, target, scheduled_for_ms, fired_at_ms, execution_id, status, error, missed";

/// Insert or replace one fire row. `INSERT OR REPLACE` is intentional: the
/// `(schedule_id, scheduled_for_ms)` key is deterministic, so a retried fire
/// (e.g. after a submission error) converges instead of duplicating.
pub async fn record(conn: &Connection, rec: &ScheduleRunRecord) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO schedule_runs (
           schedule_id, kind, target, scheduled_for_ms, fired_at_ms,
           execution_id, status, error, missed
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            rec.schedule_id.as_str(),
            rec.kind.as_str(),
            rec.target.as_str(),
            rec.scheduled_for_ms,
            rec.fired_at_ms,
            rec.execution_id.as_deref(),
            rec.status.as_str(),
            rec.error.as_deref(),
            rec.missed as i64
        ],
    )
    .await
    .context("record schedule run")?;
    Ok(())
}

/// The most recent row of `schedule_id` (latest tick wins; rowid breaks
/// same-ms ties), or `None` before its first fire.
pub async fn last(conn: &Connection, schedule_id: &str) -> Result<Option<ScheduleRunRecord>> {
    let stmt = conn
        .prepare(&format!(
            "SELECT {RUN_COLS} FROM schedule_runs
             WHERE schedule_id = ?1
             ORDER BY scheduled_for_ms DESC, rowid DESC LIMIT 1"
        ))
        .await?;
    let mut rows = stmt.query(params![schedule_id]).await?;
    match rows.next().await? {
        Some(r) => Ok(Some(row_to_record(&r)?)),
        None => Ok(None),
    }
}

/// History of `schedule_id`, newest tick first, at most `limit` rows.
pub async fn list(
    conn: &Connection,
    schedule_id: &str,
    limit: u32,
) -> Result<Vec<ScheduleRunRecord>> {
    let stmt = conn
        .prepare(&format!(
            "SELECT {RUN_COLS} FROM schedule_runs
             WHERE schedule_id = ?1
             ORDER BY scheduled_for_ms DESC, rowid DESC LIMIT ?2"
        ))
        .await?;
    let mut rows = stmt.query(params![schedule_id, limit]).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_record(&r)?);
    }
    Ok(out)
}

fn row_to_record(r: &Row) -> Result<ScheduleRunRecord> {
    Ok(ScheduleRunRecord {
        schedule_id: r.get(0)?,
        kind: r.get(1)?,
        target: r.get(2)?,
        scheduled_for_ms: r.get(3)?,
        fired_at_ms: r.get(4)?,
        execution_id: r.get(5)?,
        status: r.get(6)?,
        error: r.get(7)?,
        missed: r.get::<i64>(8)? != 0,
    })
}
