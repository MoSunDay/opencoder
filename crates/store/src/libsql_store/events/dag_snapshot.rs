use anyhow::Result;
use libsql::{params, Connection};

use crate::store::dag_snapshot::{DagStepEvent, DagStepSnapshot};

/// The caller holds the store lock. Capture a watermark, then select only
/// the latest lifecycle row per step at/below it. Large stdout payloads are
/// never materialized, and the result size depends on steps, not log volume.
pub(in crate::libsql_store) async fn read(conn: &Connection, id: &str) -> Result<DagStepSnapshot> {
    let head_seq = super::last_seq(conn, id).await?;
    let mut rows = conn
        .query(
        "WITH latest AS (SELECT max(seq) AS seq, \
           max(CASE WHEN sse_kind='step_started' \
             THEN coalesce(json_extract(payload_json, '$.at_ms'), ts) END) AS started_at_ms \
           FROM session_events WHERE session_id=?1 AND seq<=?2 \
           AND sse_kind IN ('step_started','step_done') GROUP BY json_extract(payload_json, '$.step')) \
         SELECT e.seq, sse_kind, json_extract(payload_json, '$.step'), \
         coalesce(json_extract(payload_json, '$.at_ms'), ts), \
         coalesce(json_extract(payload_json, '$.payload.ok'), 1), \
         json_extract(payload_json, '$.payload.error'), coalesce(latest.started_at_ms, 0) \
         FROM session_events e JOIN latest ON latest.seq=e.seq ORDER BY e.seq",
            params![id, head_seq],
        )
        .await?;
    let mut steps = Vec::new();
    while let Some(row) = rows.next().await? {
        steps.push(DagStepEvent {
            seq: row.get(0)?,
            started: row.get::<String>(1)? == "step_started",
            name: row.get(2)?,
            at_ms: row.get(3)?,
            ok: row.get::<i64>(4)? != 0,
            error: row.get(5)?,
            started_at_ms: row.get(6)?,
        });
    }
    Ok(DagStepSnapshot { head_seq, steps })
}
