use crate::reporting::ReportAck;
use anyhow::{ensure, Result};
use opencoder_core::brain::layered::*;
use serde_json::{json, Value};
use turso::{params, Connection};

async fn insert(
    conn: &Connection,
    id: &str,
    action: &str,
    item: &str,
    cursor: u64,
    payload: Value,
) -> Result<()> {
    ensure!(cursor <= i64::MAX as u64, "report cursor overflow");
    let key = serde_json::to_string(&(id, action, item))?;
    conn.execute("INSERT INTO execution_report_queue(key,execution_id,action,item,cursor,payload,eligible) SELECT ?1,?2,?3,?4,?5,?6,CASE WHEN ?3='layered_guidance' THEN NOT EXISTS(SELECT 1 FROM execution_report_queue WHERE execution_id=?2 AND action='layered_guidance' AND cursor<?5) ELSE 1 END WHERE NOT EXISTS(SELECT 1 FROM execution_report_acks WHERE execution_id=?2 AND action=?3 AND item=CASE WHEN ?3='layered_guidance' THEN '' ELSE ?4 END AND cursor>=?5) ON CONFLICT(execution_id,action,item) DO UPDATE SET cursor=excluded.cursor,payload=excluded.payload,eligible=excluded.eligible",params![key,id,action,item,cursor as i64,serde_json::to_string(&payload)?]).await?;
    Ok(())
}

/// Called within the same transaction as the committed Brain state.
pub(crate) async fn update(
    conn: &Connection,
    run: &LayeredRun,
    operations: &[LayeredOperation],
    events: &[LayeredEvent],
) -> Result<()> {
    let id = &run.run_id;
    conn.execute(
        "DELETE FROM execution_report_queue WHERE execution_id=?1 AND action<>'layered_guidance'",
        [id.as_str()],
    )
    .await?;
    if matches!(run.phase, LayeredPhase::Ready | LayeredPhase::Deciding) {
        insert(
            conn,
            id,
            "layered_wake",
            "",
            run.generation,
            json!({"generation":run.generation}),
        )
        .await?;
    }
    let dispatch = run.phase == LayeredPhase::Waiting
        || (run.pending_guidance
            && matches!(run.phase, LayeredPhase::Ready | LayeredPhase::Deciding));
    for op in operations {
        if dispatch && op.status == LayeredOperationStatus::Creating && !op.cancel_requested {
            insert(conn, id, "layered_dispatch", &op.operation_id, 1, json!(op)).await?;
        }
        if !run.phase.terminal() && op.cancel_requested && !op.status.terminal() {
            insert(conn, id, "layered_cancel", &op.operation_id, 1, json!(op)).await?;
        }
    }
    if run.phase.terminal() {
        conn.execute("DELETE FROM execution_report_queue WHERE execution_id=?1 AND action='layered_guidance'",[id.as_str()]).await?;
        insert(conn, id, "settle", "", run.generation, Value::Null).await?;
    } else {
        for event in events
            .iter()
            .filter(|e| e.event_type == "guidance_processed" && !e.guidance.is_empty())
        {
            insert(
                conn,
                id,
                "layered_guidance",
                &format!("{:020}", event.seq),
                event.seq,
                json!({"seq":event.seq,"guidance":event.guidance}),
            )
            .await?;
        }
    }
    conn.execute("INSERT INTO execution_report_runs(run_id,generation,last_event_seq) VALUES(?1,?2,?3) ON CONFLICT(run_id) DO UPDATE SET generation=excluded.generation,last_event_seq=excluded.last_event_seq",params![id.clone(),run.generation as i64,run.last_event_seq as i64]).await?;
    Ok(())
}

pub(crate) async fn acknowledge(conn: &Connection, ack: &ReportAck) -> Result<()> {
    ensure!(
        ack.cursor <= i64::MAX as u64,
        "report acknowledgement overflow"
    );
    // This connection is owned by the serialized writer. A previous ACK and
    // its queue deletion committed together, so duplicate ACKs need no write.
    let mut rows = conn.query(
        "SELECT cursor FROM execution_report_acks WHERE execution_id=?1 AND action=?2 AND item=?3",
        params![ack.execution_id.clone(), ack.action.clone(), ack.item.clone()],
    ).await?;
    if let Some(row) = rows.next().await? {
        if row.get::<i64>(0)? >= ack.cursor as i64 {
            return Ok(());
        }
    }
    drop(rows);
    super::super::tx::run_tx(conn,"BEGIN IMMEDIATE",||async {
        conn.execute("INSERT INTO execution_report_acks(execution_id,action,item,cursor) VALUES(?1,?2,?3,?4) ON CONFLICT(execution_id,action,item) DO UPDATE SET cursor=excluded.cursor WHERE execution_report_acks.cursor<excluded.cursor",params![ack.execution_id.clone(),ack.action.clone(),ack.item.clone(),ack.cursor as i64]).await?;
        conn.execute("DELETE FROM execution_report_queue WHERE execution_id=?1 AND action=?2 AND (?2='layered_guidance' OR item=?3) AND cursor<=?4",params![ack.execution_id.clone(),ack.action.clone(),ack.item.clone(),ack.cursor as i64]).await?;
        if ack.action == "layered_guidance" {
            conn.execute("UPDATE execution_report_queue SET eligible=1 WHERE execution_id=?1 AND action='layered_guidance' AND cursor=(SELECT MIN(cursor) FROM execution_report_queue WHERE execution_id=?1 AND action='layered_guidance')", [ack.execution_id.as_str()]).await?;
        }
        Ok(())
    }).await
}

/// One startup reconciliation repairs pre-projection data and interrupted imports.
pub(crate) async fn recover(conn: &Connection) -> Result<()> {
    loop {
        let mut rows=conn.query("SELECT r.run_id FROM brain_layered_runs r LEFT JOIN execution_report_runs p ON p.run_id=r.run_id WHERE p.run_id IS NULL OR p.generation<>r.generation LIMIT 128",()).await?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next().await? {
            ids.push(row.get::<String>(0)?);
        }
        if ids.is_empty() {
            break;
        }
        drop(rows);
        super::super::tx::run_tx(conn,"BEGIN IMMEDIATE",||async {
            for id in &ids {
                let Some(snapshot)=super::super::brain_layered::load(conn,id).await? else {continue};
                let mut rows=conn.query("SELECT body FROM brain_layered_events WHERE run_id=?1 AND json_extract(body,'$.event_type')='guidance_processed' ORDER BY seq",[id.as_str()]).await?;
                let mut events=Vec::new();while let Some(row)=rows.next().await? {events.push(serde_json::from_str(&row.get::<String>(0)?)?);}
                update(conn,&snapshot.run,&snapshot.operations,&events).await?;
            }
            Ok(())
        }).await?;
    }
    Ok(())
}
