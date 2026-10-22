use crate::reporting::{PendingReport, ReportAck, ReportRunState};
use anyhow::Result;
use turso::{params, Connection};

pub(crate) async fn has_pending(conn: &Connection) -> Result<bool> {
    Ok(conn
        .query("SELECT 1 FROM execution_report_queue LIMIT 1", ())
        .await?
        .next()
        .await?
        .is_some())
}

pub(crate) async fn pending(
    conn: &Connection,
    after: Option<&str>,
    limit: u32,
) -> Result<Vec<PendingReport>> {
    let limit = limit.clamp(1, 64) as i64;
    let mut result = Vec::new();
    // Two indexed ranges wrap the cursor without sorting the whole queue.
    for wrap in [false, true] {
        if wrap && (after.is_none() || result.len() >= limit as usize) {
            break;
        }
        let comparison = if wrap { "q.key<=?1" } else { "q.key>?1" };
        let sql=format!("SELECT q.key,q.execution_id,q.action,q.item,q.cursor,q.payload FROM execution_report_queue q WHERE q.eligible=1 AND {comparison} ORDER BY q.key LIMIT ?2");
        let mut rows = conn
            .query(
                &sql,
                params![after.unwrap_or(""), limit - result.len() as i64],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            result.push(PendingReport {
                key: row.get(0)?,
                execution_id: row.get(1)?,
                action: row.get(2)?,
                item: row.get(3)?,
                cursor: row.get::<i64>(4)? as u64,
                payload: serde_json::from_str(&row.get::<String>(5)?)?,
            });
        }
    }
    Ok(result)
}
pub(crate) async fn acks(conn: &Connection) -> Result<Vec<ReportAck>> {
    let mut rows = conn
        .query(
            "SELECT execution_id,action,item,cursor FROM execution_report_acks",
            (),
        )
        .await?;
    let mut result = Vec::new();
    while let Some(row) = rows.next().await? {
        result.push(ReportAck {
            execution_id: row.get(0)?,
            action: row.get(1)?,
            item: row.get(2)?,
            cursor: row.get::<i64>(3)? as u64,
        });
    }
    Ok(result)
}
pub(crate) async fn state(conn: &Connection, id: &str) -> Result<Option<ReportRunState>> {
    let mut rows = conn
        .query(
            "SELECT generation,last_event_seq FROM execution_report_runs WHERE run_id=?1",
            [id],
        )
        .await?;
    rows.next()
        .await?
        .map(|row| {
            Ok(ReportRunState {
                generation: row.get::<i64>(0)? as u64,
                last_event_seq: row.get::<i64>(1)? as u64,
            })
        })
        .transpose()
}
pub(crate) async fn operation(
    conn: &Connection,
    id: &str,
    operation: &str,
) -> Result<Option<opencoder_core::brain::layered::LayeredOperation>> {
    let mut rows = conn
        .query(
            "SELECT body FROM brain_layered_operations WHERE run_id=?1 AND operation_id=?2",
            params![id, operation],
        )
        .await?;
    rows.next()
        .await?
        .map(|row| Ok(serde_json::from_str(&row.get::<String>(0)?)?))
        .transpose()
}
