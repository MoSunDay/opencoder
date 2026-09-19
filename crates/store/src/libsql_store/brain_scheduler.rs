//! All v3 projection changes commit under the owning Store's database lock.
use anyhow::{ensure, Context, Result};
use libsql::{params, Connection};
use opencoder_core::brain::*;
mod schema;
pub(super) use schema::initialize;

pub async fn load(conn: &Connection, id: &str) -> Result<Option<BrainSchedulerSnapshot>> {
    let mut rows = conn
        .query(
            "SELECT body FROM brain_scheduler_runs WHERE run_id=?1",
            [id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let run: BrainSchedulerRun = serde_json::from_str(&row.get::<String>(0)?)?;
    let mut rows=conn.query("SELECT body FROM brain_scheduler_operations WHERE run_id=?1 ORDER BY round,execution_id",[id]).await?;
    let mut operations = vec![];
    while let Some(row) = rows.next().await? {
        operations.push(serde_json::from_str(&row.get::<String>(0)?)?);
    }
    Ok(Some(BrainSchedulerSnapshot {
        schema_version: 3,
        run,
        operations,
    }))
}
pub async fn commit(
    conn: &Connection,
    change: &BrainSchedulerChange,
) -> Result<BrainSchedulerSnapshot> {
    super::tx::run_tx(conn,"BEGIN IMMEDIATE",||async{
        let old=load(conn,&change.run.run_id).await?;
        ensure!(old.as_ref().map(|s|s.run.generation)==change.expected_generation,"brain scheduler generation conflict");
        ensure!(change.run.generation==change.expected_generation.map_or(0,|v|v+1),"invalid next generation");
        if let Some(old)=&old{
            ensure!(!old.run.phase.terminal() || change.run.phase==old.run.phase,"terminal run cannot change phase");
            ensure!(old.operations.iter().all(|o|change.operations.iter().any(|n|n.operation_id==o.operation_id && n.run_id==o.run_id && n.round==o.round && n.capability_id==o.capability_id && n.execution_kind==o.execution_kind && n.execution_id==o.execution_id && (!o.status.terminal() || o.status==n.status))),"operation identity or terminal state changed");
        }
        let mut run=change.run.clone();run.last_event_seq=old.as_ref().map_or(0,|s|s.run.last_event_seq);
        conn.execute("INSERT INTO brain_scheduler_runs(run_id,generation,body) VALUES(?1,?2,?3) ON CONFLICT(run_id) DO UPDATE SET generation=excluded.generation,body=excluded.body",params![run.run_id.clone(),run.generation as i64,serde_json::to_string(&run)?]).await?;
        for op in &change.operations{
            ensure!(op.run_id==run.run_id,"foreign operation");
            conn.execute("INSERT INTO brain_scheduler_operations(execution_id,operation_id,run_id,round,body) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(execution_id) DO UPDATE SET body=excluded.body WHERE run_id=excluded.run_id AND operation_id=excluded.operation_id",params![op.execution_id.clone(),op.operation_id.clone(),op.run_id.clone(),op.round as i64,serde_json::to_string(op)?]).await?;
        }
        for e in &change.events{
            ensure!(e.run_id==run.run_id,"foreign scheduler event");
            ensure!(e.reason_summary.as_ref().is_none_or(|s|s.chars().count()<=1024) && e.decision_summary.as_ref().is_none_or(|s|s.len()<=128),"scheduler summaries exceed bounds");
            let mut e=e.clone();run.last_event_seq+=1;e.seq=run.last_event_seq;
            // Unique terminal source key also protects against callers that
            // accidentally replay a change with a newer generation.
            conn.execute("INSERT INTO brain_scheduler_events(run_id,seq,execution_id,source_sequence,body) VALUES(?1,?2,?3,?4,?5)",params![run.run_id.clone(),e.seq as i64,e.execution_id.clone(),e.source_sequence.map(|v|v as i64),serde_json::to_string(&e)?]).await?;
        }
        conn.execute("UPDATE brain_scheduler_runs SET body=?1 WHERE run_id=?2",params![serde_json::to_string(&run)?,run.run_id.clone()]).await?;
        load(conn,&run.run_id).await?.context("scheduler projection disappeared")
    }).await
}
pub async fn page(
    conn: &Connection,
    id: &str,
    after: u64,
    limit: u32,
) -> Result<Vec<BrainSchedulerEvent>> {
    let mut rows=conn.query("SELECT body FROM brain_scheduler_events WHERE run_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3",params![id,after as i64,limit.clamp(1,500)]).await?;
    let mut events = vec![];
    while let Some(row) = rows.next().await? {
        events.push(serde_json::from_str(&row.get::<String>(0)?)?);
    }
    Ok(events)
}
