//! DAG event stream (`dag_events`) — the node-uploaded, append-only log the
//! browser UI projects step state from. The server keeps NO per-step
//! scheduling state: it only stores and replays these rows.

use anyhow::{Context, Result};
use libsql::{params, Connection};

use crate::types::DagEventRecord;

/// Shared with [`super::dag`] so the in-transaction synthetic
/// `run_finished` inserts (finalize / lost-sweep) reuse the exact same SQL
/// instead of duplicating it.
pub(super) const INSERT_EVENT: &str =
    "INSERT INTO dag_events (run_id, kind, step, payload, at_ms) VALUES (?, ?, ?, ?, ?)";

/// Persist a batch of node-uploaded events in one transaction through ONE
/// prepared INSERT. `seq` is a global AUTOINCREMENT counter, so the rows just
/// inserted are exactly the N highest seqs (the tx holds the write lock); one
/// backfill SELECT returns them, reversed into ascending emission order.
pub async fn append_events(conn: &Connection, events: &[DagEventRecord]) -> Result<Vec<i64>> {
    if events.is_empty() {
        return Ok(Vec::new());
    }
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let stmt = conn
            .prepare(INSERT_EVENT)
            .await
            .context("prepare dag_event insert")?;
        for ev in events {
            let payload =
                serde_json::to_string(&ev.payload).context("serialize dag event payload")?;
            stmt.execute(params![
                ev.run_id.as_str(),
                ev.kind.as_str(),
                ev.step.as_deref(),
                payload,
                ev.at_ms
            ])
            .await
            .context("insert dag_event in tx")?;
            // libsql's local `step` has no auto-reset: without an explicit
            // reset the next bind silently reuses the exhausted statement's
            // previous bindings (every row duplicates the first).
            stmt.reset();
        }
        drop(stmt);
        let stmt = conn
            .prepare("SELECT seq FROM dag_events ORDER BY seq DESC LIMIT ?1")
            .await?;
        let mut rows = stmt.query(params![events.len() as i64]).await?;
        let mut seqs = Vec::with_capacity(events.len());
        while let Some(r) = rows.next().await? {
            seqs.push(r.get::<Option<i64>>(0)?.unwrap_or(0));
        }
        drop(rows);
        drop(stmt);
        seqs.reverse();
        Ok(seqs)
    })
    .await
}

/// Replay slice for the run SSE: `seq > after`, ascending, bounded.
pub async fn events_after(
    conn: &Connection,
    run_id: &str,
    after: i64,
    limit: u32,
) -> Result<Vec<DagEventRecord>> {
    super::dag::collect(
        conn,
        "SELECT seq, run_id, kind, step, payload, at_ms FROM dag_events
         WHERE run_id = ?1 AND seq > ?2 ORDER BY seq ASC LIMIT ?3",
        params![run_id, after, i64::from(limit)],
        row_to_event,
    )
    .await
}

fn row_to_event(r: &libsql::Row) -> Result<DagEventRecord> {
    let seq: i64 = r.get(0)?;
    let run_id: String = r.get(1)?;
    let payload_json: String = r.get(4)?;
    let payload = serde_json::from_str::<serde_json::Value>(&payload_json).unwrap_or_else(|e| {
        tracing::warn!(run_id, seq, error = %e, "bad dag event payload, using null");
        serde_json::Value::Null
    });
    Ok(DagEventRecord {
        seq: Some(seq),
        run_id,
        kind: r.get(2)?,
        step: r.get(3)?,
        payload,
        at_ms: r.get(5)?,
    })
}
