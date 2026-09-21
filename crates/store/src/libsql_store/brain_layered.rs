//! All v4 projection changes commit under the owning Store's database lock.
use anyhow::{ensure, Context, Result};
use libsql::{params, Connection};
use opencoder_core::brain::layered::*;
mod schema;
pub(super) use schema::initialize;

pub async fn load(conn: &Connection, id: &str) -> Result<Option<LayeredSnapshot>> {
    let mut rows = conn
        .query("SELECT body FROM brain_layered_runs WHERE run_id=?1", [id])
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let run: LayeredRun = serde_json::from_str(&row.get::<String>(0)?)?;
    let mut rows = conn
        .query(
            "SELECT body FROM brain_layered_operations WHERE run_id=?1 ORDER BY layer,operation_id",
            [id],
        )
        .await?;
    let mut operations = vec![];
    while let Some(row) = rows.next().await? {
        operations.push(serde_json::from_str(&row.get::<String>(0)?)?);
    }
    Ok(Some(LayeredSnapshot {
        schema_version: LAYERED_SCHEMA_VERSION,
        run,
        operations,
    }))
}

pub async fn commit(conn: &Connection, change: &LayeredChange) -> Result<LayeredSnapshot> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async {
        let old = load(conn, &change.run.run_id).await?;
        ensure!(
            old.as_ref().map(|s| s.run.generation) == change.expected_generation,
            "layered run generation conflict"
        );
        ensure!(
            change.run.generation == change.expected_generation.map_or(0, |v| v + 1),
            "invalid next generation"
        );
        if let Some(old) = &old {
            ensure!(
                !old.run.phase.terminal() || change.run.phase == old.run.phase,
                "terminal run cannot change phase"
            );
            ensure!(
                old.operations.iter().all(|o| {
                    change.operations.iter().any(|n| {
                        n.operation_id == o.operation_id
                            && n.run_id == o.run_id
                            && n.layer == o.layer
                            && n.node_id == o.node_id
                            && n.attempt == o.attempt
                            && n.capability_id == o.capability_id
                            && n.execution_kind == o.execution_kind
                            && n.execution_id == o.execution_id
                            && (!o.status.terminal() || o.status == n.status)
                    })
                }),
                "operation identity or terminal state changed"
            );
        }
        let mut run = change.run.clone();
        run.last_event_seq = old.as_ref().map_or(0, |s| s.run.last_event_seq);
        conn.execute("INSERT INTO brain_layered_runs(run_id,generation,body) VALUES(?1,?2,?3) ON CONFLICT(run_id) DO UPDATE SET generation=excluded.generation,body=excluded.body",params![run.run_id.clone(),run.generation as i64,serde_json::to_string(&run)?]).await?;
        for op in &change.operations {
            ensure!(op.run_id == run.run_id, "foreign operation");
            ensure!(
                op.layer == run.layer || op.status.terminal(),
                "operation layer is ahead of the run"
            );
            conn.execute("INSERT INTO brain_layered_operations(execution_id,operation_id,run_id,layer,body) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(execution_id) DO UPDATE SET body=excluded.body WHERE run_id=excluded.run_id AND operation_id=excluded.operation_id",params![op.execution_id.clone(),op.operation_id.clone(),op.run_id.clone(),op.layer as i64,serde_json::to_string(op)?]).await?;
        }
        for e in &change.events {
            ensure!(e.run_id == run.run_id, "foreign layered event");
            ensure!(
                e.reason_summary
                    .as_ref()
                    .is_none_or(|s| s.chars().count() <= 1024)
                    && e.decision_summary.as_ref().is_none_or(|s| s.len() <= 128),
                "layered summaries exceed bounds"
            );
            let mut e = e.clone();
            run.last_event_seq += 1;
            e.seq = run.last_event_seq;
            conn.execute("INSERT INTO brain_layered_events(run_id,seq,execution_id,source_sequence,body) VALUES(?1,?2,?3,?4,?5)",params![run.run_id.clone(),e.seq as i64,e.execution_id.clone(),e.source_sequence.map(|v|v as i64),serde_json::to_string(&e)?]).await?;
        }
        conn.execute(
            "UPDATE brain_layered_runs SET body=?1 WHERE run_id=?2",
            params![serde_json::to_string(&run)?, run.run_id.clone()],
        )
        .await?;
        load(conn, &run.run_id)
            .await?
            .context("layered projection disappeared")
    })
    .await
}

pub async fn page(
    conn: &Connection,
    id: &str,
    after: u64,
    limit: u32,
) -> Result<Vec<LayeredEvent>> {
    let mut rows = conn.query("SELECT body FROM brain_layered_events WHERE run_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3",params![id,after as i64,limit.clamp(1,500)]).await?;
    let mut events = vec![];
    while let Some(row) = rows.next().await? {
        events.push(serde_json::from_str(&row.get::<String>(0)?)?);
    }
    Ok(events)
}
