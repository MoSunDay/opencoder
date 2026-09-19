//! Schedule persistence: the DEFINITIONS table (`schedules`, schema v27 —
//! the source of truth for the control-plane cron scheduler; the legacy
//! `schedules.json` is only a bootstrap seed) and the fire ledger
//! (`schedule_runs`, v26 — the audit trail).
//!
//! Free functions over a raw `Connection`, mirroring sibling submodules
//! (`team_runs.rs` / `dag.rs`). The DDL constants live here (not in
//! `schema.rs`) so the domain owns its tables; `schema.rs` imports and
//! registers them in the bootstrap batch + migrations.

use anyhow::{Context, Result};
use libsql::{params, Connection, Row};

use crate::schedule_types::{ScheduleDefRecord, ScheduleRunRecord};

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

// ----------------- Schedule DEFINITIONS (`schedules`, v27) ---------------

/// Definition DDL registered by `schema.rs` (bootstrap batch + v27
/// migration). No FK to `schedule_runs`: the ledger intentionally outlives
/// its definitions (same contract as `dag_runs`), so deleting a schedule
/// keeps its fire history queryable.
pub(super) const CREATE_SCHEDULES: &str = "\
CREATE TABLE IF NOT EXISTS schedules (
  id TEXT PRIMARY KEY,
  cron TEXT NOT NULL,
  timezone TEXT,
  enabled INTEGER NOT NULL DEFAULT 1,
  kind TEXT NOT NULL,
  target TEXT NOT NULL,
  params_json TEXT NOT NULL DEFAULT '{}',
  overlap TEXT NOT NULL DEFAULT 'skip',
  node_id TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
)";

const DEF_COLS: &str =
    "id, cron, timezone, enabled, kind, target, params_json, overlap, node_id, created_at, updated_at";

/// Insert or update one definition by id. `created_at` is stamped on the
/// first insert and PRESERVED on conflict (the API never lets a client
/// rewrite creation metadata); `updated_at` always moves to `now_ms`.
pub async fn upsert_def(
    conn: &Connection,
    job: &opencoder_core::config::ScheduleJob,
    now_ms: i64,
) -> Result<()> {
    let params_json = serde_json::to_string(&job.params).context("serialize schedule params")?;
    conn.execute(
        "INSERT INTO schedules (
           id, cron, timezone, enabled, kind, target, params_json, overlap, node_id,
           created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(id) DO UPDATE SET
           cron = excluded.cron, timezone = excluded.timezone, enabled = excluded.enabled,
           kind = excluded.kind, target = excluded.target, params_json = excluded.params_json,
           overlap = excluded.overlap, node_id = excluded.node_id, updated_at = excluded.updated_at",
        params![
            job.id.as_str(),
            job.cron.as_str(),
            job.timezone.as_deref(),
            job.enabled as i64,
            job.kind.as_str(),
            job.target.as_str(),
            params_json.as_str(),
            job.overlap.as_str(),
            job.node_id.as_deref(),
            now_ms,
            now_ms,
        ],
    )
    .await
    .context("upsert schedule")?;
    Ok(())
}

/// One definition by id (with its timestamps), or `None`.
pub async fn get_def(conn: &Connection, id: &str) -> Result<Option<ScheduleDefRecord>> {
    let stmt = conn
        .prepare(&format!(
            "SELECT {DEF_COLS} FROM schedules WHERE id = ?1 LIMIT 1"
        ))
        .await?;
    let mut rows = stmt.query(params![id]).await?;
    match rows.next().await? {
        Some(r) => Ok(Some(row_to_def(&r)?)),
        None => Ok(None),
    }
}

/// Every definition, stable `id` order (the list surface and the scheduler
/// scan iterate it; order must not depend on insert history).
pub async fn list_defs(conn: &Connection) -> Result<Vec<ScheduleDefRecord>> {
    let stmt = conn
        .prepare(&format!("SELECT {DEF_COLS} FROM schedules ORDER BY id ASC"))
        .await?;
    let mut rows = stmt.query(()).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_def(&r)?);
    }
    Ok(out)
}

/// Delete one definition. Its `schedule_runs` ledger rows are KEPT (no FK):
/// fire history is an audit trail and outlives the definition.
pub async fn delete_def(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM schedules WHERE id = ?1", params![id])
        .await
        .context("delete schedule")?;
    Ok(())
}

fn row_to_def(r: &Row) -> Result<ScheduleDefRecord> {
    let kind: String = r.get(4)?;
    let overlap: String = r.get(7)?;
    let params_json: String = r.get(6)?;
    let id: String = r.get(0)?;
    Ok(ScheduleDefRecord {
        job: opencoder_core::config::ScheduleJob {
            id: id.clone(),
            cron: r.get(1)?,
            timezone: r.get(2)?,
            enabled: r.get::<i64>(3)? != 0,
            kind: opencoder_core::config::ScheduleKind::parse(&kind)
                .with_context(|| format!("parse schedule kind {kind:?}"))?,
            target: r.get(5)?,
            params: serde_json::from_str(&params_json)
                .with_context(|| format!("parse schedule params json for {id}"))?,
            overlap: opencoder_core::config::ScheduleOverlap::parse(&overlap)
                .with_context(|| format!("parse schedule overlap {overlap:?}"))?,
            node_id: r.get(8)?,
        },
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}
