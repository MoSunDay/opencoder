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
            "SELECT seq, sse_kind, json_extract(payload_json, '$.step'), \
         coalesce(json_extract(payload_json, '$.at_ms'), ts), \
         coalesce(json_extract(payload_json, '$.payload.ok'), 1), \
         json_extract(payload_json, '$.payload.error') FROM session_events \
         WHERE seq IN (SELECT max(seq) FROM session_events \
           WHERE session_id=?1 AND seq<=?2 AND sse_kind IN ('step_started','step_done') \
           GROUP BY json_extract(payload_json, '$.step')) ORDER BY seq",
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
        });
    }
    Ok(DagStepSnapshot { head_seq, steps })
}
