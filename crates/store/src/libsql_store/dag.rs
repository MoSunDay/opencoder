//! DAG workflow store (`dag_defs` / `dag_runs`) — definitions and the
//! dispatched-run lifecycle: dispatch, FIFO claim, transitions, cancel,
//! lost-node convergence. Free functions over a raw `Connection`, mirroring
//! sibling submodules; multi-statement ops run via [`super::tx::run_tx`]
//! (`BEGIN IMMEDIATE`). Claim / cancel-piggyback / lost-sweep are
//! transplanted from `node_tasks` (grid: `opencoder_dag::transitions`).
//! The append-only event stream lives in [`super::dag_events`].

use anyhow::{Context, Result};
use libsql::{params, params::IntoParams, Connection};
use opencoder_dag::{transition_allowed, DagRunStatus};

use crate::types::{DagDefRecord, DagRunRecord};

const DEF_COLS: &str = "id, name, spec_json, created_at, updated_at";
const RUN_COLS: &str =
    "id, dag_id, name, spec_json, node_id, status, error, created_at, claimed_at, finished_at";
/// Bounded CAS retries in [`claim_next`]: `BEGIN IMMEDIATE` already excludes
/// racing writers, so a zero-row UPDATE is near-impossible — defensive only.
const CLAIM_ATTEMPTS: usize = 3;

/// Upsert keyed by `spec.name` (the conflict target): a re-publish replaces
/// `spec_json`/`updated_at` while the FIRST row's `id`/`created_at` stay put.
pub async fn upsert_def(conn: &Connection, def: &DagDefRecord) -> Result<()> {
    exec(
        conn,
        "INSERT INTO dag_defs (id, name, spec_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(name) DO UPDATE SET spec_json = excluded.spec_json, updated_at = excluded.updated_at",
        params![
            def.id.as_str(),
            def.name.as_str(),
            def.spec_json.as_str(),
            def.created_at,
            def.updated_at
        ],
        "upsert dag_def",
    )
    .await?;
    Ok(())
}

pub async fn list_defs(conn: &Connection) -> Result<Vec<DagDefRecord>> {
    collect(
        conn,
        &format!("SELECT {DEF_COLS} FROM dag_defs ORDER BY name ASC"),
        (),
        row_to_def,
    )
    .await
}

pub async fn get_def(conn: &Connection, id: &str) -> Result<Option<DagDefRecord>> {
    first(
        conn,
        &format!("SELECT {DEF_COLS} FROM dag_defs WHERE id = ?1 LIMIT 1"),
        params![id],
        row_to_def,
    )
    .await
}

pub async fn delete_def(conn: &Connection, id: &str) -> Result<()> {
    exec(
        conn,
        "DELETE FROM dag_defs WHERE id = ?1",
        params![id],
        "delete dag_def",
    )
    .await?;
    Ok(())
}

/// Enqueue a run — always fresh: `status` is forced to `pending` and
/// `error`/`claimed_at`/`finished_at` to NULL no matter what the caller's
/// record carries. No synthetic session (unlike node_tasks): the `spec_json`
/// snapshot carries all execution context.
pub async fn dispatch(conn: &Connection, run: &DagRunRecord) -> Result<DagRunRecord> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        exec(
            conn,
            "INSERT INTO dag_runs (id, dag_id, name, spec_json, node_id, status, error, created_at, claimed_at, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', NULL, ?6, NULL, NULL)",
            params![
                run.id.as_str(),
                run.dag_id.as_str(),
                run.name.as_str(),
                run.spec_json.as_str(),
                run.node_id.as_deref(),
                run.created_at
            ],
            "insert dag_run",
        )
        .await?;
        get_run(conn, &run.id).await?
            .ok_or_else(|| anyhow::anyhow!("dispatch_dag_run: row {} vanished", run.id))
    })
    .await
}

/// Atomically claim the oldest pending run this node may take (pinned to it
/// or unpinned). Returns `None` when the node already has a `running` run
/// (single-active-run policy) or nothing is due. The CAS
/// `UPDATE ... WHERE status='pending'` (never RETURNING) makes losing racers
/// see zero rows; they retry the scan a bounded few times, then give up.
pub async fn claim_next(
    conn: &Connection,
    node_id: &str,
    now_ms: i64,
) -> Result<Option<DagRunRecord>> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        for _ in 0..CLAIM_ATTEMPTS {
            // (a) Single active run per node.
            let busy = first(
                conn,
                "SELECT 1 FROM dag_runs WHERE node_id = ?1 AND status = 'running' LIMIT 1",
                params![node_id],
                |r| Ok(r.get::<i64>(0)?),
            )
            .await?;
            if busy.is_some() {
                return Ok(None);
            }
            // (b) Oldest eligible pending run (FIFO). `created_at` has
            // millisecond resolution, so same-ms dispatches tie; the implicit
            // insertion-order `rowid` is the tiebreak (ULIDs are NOT
            // monotonic and must never order anything).
            let next_id = first(
                conn,
                "SELECT id FROM dag_runs WHERE status = 'pending' AND (node_id IS NULL OR node_id = ?1)
                 ORDER BY created_at ASC, rowid ASC LIMIT 1",
                params![node_id],
                |r| Ok(r.get::<String>(0)?),
            )
            .await?;
            let Some(run_id) = next_id else {
                return Ok(None);
            };
            // (c) Compare-and-swap pending -> running; an unpinned row gets
            // its `node_id` stamped here.
            let changed = exec(
                conn,
                "UPDATE dag_runs SET status = 'running', node_id = ?2, claimed_at = ?3
                 WHERE id = ?1 AND status = 'pending'",
                params![run_id.as_str(), node_id, now_ms],
                "claim dag_run",
            )
            .await?;
            if changed == 0 {
                continue; // lost the race — retry the scan
            }
            return get_run(conn, &run_id).await;
        }
        Ok(None)
    })
    .await
}

/// Apply a validated transition atomically; terminal writes stamp
/// `finished_at`. The grid already rejects every same-state no-op (terminal
/// states freeze, live states never self-loop).
pub async fn update_status(
    conn: &Connection,
    run_id: &str,
    to: DagRunStatus,
    error: Option<&str>,
    now_ms: i64,
) -> Result<DagRunRecord> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let from = current_status(conn, run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("update_dag_run_status: dag run {run_id} not found"))?;
        if !transition_allowed(from, to) {
            anyhow::bail!("illegal dag run transition {from} -> {to} for run {run_id}");
        }
        let finished_at = if to.is_terminal() { Some(now_ms) } else { None };
        exec(
            conn,
            "UPDATE dag_runs SET status = ?2, error = ?3, finished_at = ?4 WHERE id = ?1",
            params![run_id, to.as_str(), error, finished_at],
            "update dag_run status",
        )
        .await?;
        get_run(conn, run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("update_dag_run_status: dag run {run_id} vanished"))
    })
    .await
}

/// Request cancellation: `pending` collapses straight to `cancelled`
/// (nothing claimed it yet), `running` flips to `cancelling` for the node to
/// observe via the heartbeat piggyback, `cancelling` is an idempotent no-op,
/// terminal states bail (upstream maps to 409).
pub async fn cancel(conn: &Connection, run_id: &str, now_ms: i64) -> Result<()> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let from = current_status(conn, run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("cancel_dag_run: dag run {run_id} not found"))?;
        match from {
            DagRunStatus::Pending => {
                exec(
                    conn,
                    "UPDATE dag_runs SET status = 'cancelled', finished_at = ?2 WHERE id = ?1",
                    params![run_id, now_ms],
                    "cancel pending dag_run",
                )
                .await?;
            }
            DagRunStatus::Running => {
                exec(
                    conn,
                    "UPDATE dag_runs SET status = 'cancelling' WHERE id = ?1",
                    params![run_id],
                    "cancel running dag_run",
                )
                .await?;
            }
            DagRunStatus::Cancelling => {} // already requested — idempotent
            terminal => {
                anyhow::bail!("cancel_dag_run: dag run {run_id} already terminal ({terminal})")
            }
        }
        Ok(())
    })
    .await
}

/// `cancelling` runs of `node_id` — the heartbeat piggyback payload.
pub async fn cancelling_runs(conn: &Connection, node_id: &str) -> Result<Vec<String>> {
    collect(
        conn,
        "SELECT id FROM dag_runs WHERE node_id = ?1 AND status = 'cancelling'
         ORDER BY created_at ASC, rowid ASC",
        params![node_id],
        |r| Ok(r.get::<String>(0)?),
    )
    .await
}

pub async fn get_run(conn: &Connection, run_id: &str) -> Result<Option<DagRunRecord>> {
    first(
        conn,
        &format!("SELECT {RUN_COLS} FROM dag_runs WHERE id = ?1 LIMIT 1"),
        params![run_id],
        row_to_run,
    )
    .await
}

/// Newest-first listing (`created_at DESC, rowid DESC`) — the exact reverse
/// of the claim FIFO order.
pub async fn list_runs(conn: &Connection, limit: u32) -> Result<Vec<DagRunRecord>> {
    collect(
        conn,
        &format!("SELECT {RUN_COLS} FROM dag_runs ORDER BY created_at DESC, rowid DESC LIMIT ?1"),
        params![i64::from(limit)],
        row_to_run,
    )
    .await
}

async fn current_status(conn: &Connection, run_id: &str) -> Result<Option<DagRunStatus>> {
    let s: Option<String> = first(
        conn,
        "SELECT status FROM dag_runs WHERE id = ?1 LIMIT 1",
        params![run_id],
        |r| Ok(r.get::<String>(0)?),
    )
    .await?;
    let Some(s) = s else { return Ok(None) };
    DagRunStatus::parse(&s)
        .map(Some)
        .with_context(|| format!("parse dag run status {s:?} for run {run_id}"))
}

/// Collapse zombie runs of lost nodes: every `running`/`cancelling` run of a
/// node whose heartbeat is older than `stale_ms` becomes `error("node lost")`
/// stamped `finished_at`, in one transaction. Exclusive age bound (exactly
/// `stale_ms` old is still fresh), matching
/// [`super::node_tasks::converge_lost`]; `pending` rows never block claiming
/// and terminal rows are frozen, so the sweep is idempotent.
pub async fn converge_lost(
    conn: &Connection,
    now_ms: i64,
    stale_ms: i64,
) -> Result<Vec<DagRunRecord>> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        // (a) Snapshot candidate ids: active runs of heartbeat-stale nodes
        // (unclaimed rows have NULL node_id and drop out of the JOIN).
        let ids = collect(
            conn,
            "SELECT dag_runs.id FROM dag_runs JOIN nodes ON dag_runs.node_id = nodes.id
             WHERE (?1 - nodes.last_seen_at) > ?2 AND dag_runs.status IN ('running','cancelling')",
            params![now_ms, stale_ms],
            |r| Ok(r.get::<String>(0)?),
        )
        .await?;
        // (b) Conditional CAS per row (no RETURNING, same as claim).
        for id in &ids {
            let changed = exec(
                conn,
                "UPDATE dag_runs SET status = 'error', error = 'node lost', finished_at = ?2
                 WHERE id = ?1 AND status IN ('running','cancelling')",
                params![id.as_str(), now_ms],
                "converge lost dag_run",
            )
            .await?;
            if changed == 0 {
                continue; // raced away between select and update
            }
        }
        // (c) Read the converged rows back in selection order.
        let mut out = Vec::new();
        for id in &ids {
            if let Some(record) = get_run(conn, id).await? {
                out.push(record);
            }
        }
        Ok(out)
    })
    .await
}

// ── helpers / row mapping ─────────────────────────────────────────────────

/// `execute` + `context` — the shared write-half boilerplate.
async fn exec(
    conn: &Connection,
    sql: &str,
    params: impl IntoParams,
    ctx: &'static str,
) -> Result<u64> {
    conn.execute(sql, params).await.context(ctx)
}

/// Prepare + query + map every row — the shared read-half boilerplate
/// (also reused by [`super::dag_events`]).
pub(super) async fn collect<T>(
    conn: &Connection,
    sql: &str,
    params: impl IntoParams,
    map: impl Fn(&libsql::Row) -> Result<T>,
) -> Result<Vec<T>> {
    let stmt = conn.prepare(sql).await?;
    let mut rows = stmt.query(params).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(map(&r)?);
    }
    Ok(out)
}

/// [`collect`]'s LIMIT-1 sibling.
async fn first<T>(
    conn: &Connection,
    sql: &str,
    params: impl IntoParams,
    map: impl Fn(&libsql::Row) -> Result<T>,
) -> Result<Option<T>> {
    let stmt = conn.prepare(sql).await?;
    let mut rows = stmt.query(params).await?;
    match rows.next().await? {
        Some(r) => Ok(Some(map(&r)?)),
        None => Ok(None),
    }
}

fn row_to_def(r: &libsql::Row) -> Result<DagDefRecord> {
    Ok(DagDefRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        spec_json: r.get(2)?,
        created_at: r.get(3)?,
        updated_at: r.get(4)?,
    })
}

fn row_to_run(r: &libsql::Row) -> Result<DagRunRecord> {
    let s = r.get::<String>(5)?;
    Ok(DagRunRecord {
        id: r.get(0)?,
        dag_id: r.get(1)?,
        name: r.get(2)?,
        spec_json: r.get(3)?,
        node_id: r.get(4)?,
        status: DagRunStatus::parse(&s).with_context(|| format!("parse dag run status {s:?}"))?,
        error: r.get(6)?,
        created_at: r.get(7)?,
        claimed_at: r.get(8)?,
        finished_at: r.get(9)?,
    })
}
