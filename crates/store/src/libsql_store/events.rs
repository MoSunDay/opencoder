use anyhow::{Context, Result};
use libsql::{params, Connection};

use crate::types::{EventKind, SessionEventPage, SessionEventRecord};

const EVENT_OVERHEAD_BYTES: usize = 512;

const INSERT_EVENT: &str = "\
INSERT INTO session_events (session_id, type, payload_json, sse_kind, ts)
VALUES (?, ?, ?, ?, ?)";

/// Persist a batch of events in a single transaction. The seqs assigned to
/// the just-inserted rows are back-filled with ONE `SELECT` at the end (the N
/// highest seqs for the session — AUTOINCREMENT assigns contiguous ids inside
/// the transaction and the write lock prevents concurrent interleave). Returns
/// the seqs in input (emission) order. All `events` must share `session_id`.
pub async fn append_many(conn: &Connection, events: &[SessionEventRecord]) -> Result<Vec<i64>> {
    if events.is_empty() {
        return Ok(Vec::new());
    }
    let session_id = events[0].session_id.as_str();
    // Batch backfill assumes a single session: it captures `session_id` once
    // and reads back the top-N seqs for THAT session. If callers mix session
    // ids in one batch, the rows get inserted under their own ids but the
    // returned seqs are computed for only `events[0]`'s session — silently
    // misaligned with no error. Reject the mixed batch up front instead.
    for ev in events {
        if ev.session_id != session_id {
            anyhow::bail!("append_events: all events in a batch must share the same session_id");
        }
    }
    super::tx::run_tx(conn, "BEGIN", || async move {
        for ev in events {
            let payload_json =
                serde_json::to_string(&ev.payload).context("serialize event payload")?;
            conn.execute(
                INSERT_EVENT,
                params![
                    ev.session_id.as_str(),
                    kind_str(ev.kind),
                    payload_json,
                    ev.sse_kind.as_deref(),
                    ev.ts
                ],
            )
            .await
            .context("insert event in tx")?;
        }
        // Batch backfill: the rows we just inserted are the top-N seqs for this
        // session (the tx holds the write lock, so no concurrent writer can slip
        // in between our inserts and this read). Fetch them newest-first, then
        // reverse into emission order.
        let n = events.len() as i64;
        let stmt = conn
            .prepare(
                "SELECT seq FROM session_events WHERE session_id = ? ORDER BY seq DESC LIMIT ?",
            )
            .await?;
        let mut rows = stmt.query(params![session_id, n]).await?;
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

pub async fn last_seq(conn: &Connection, session_id: &str) -> Result<i64> {
    let stmt = conn
        .prepare("SELECT MAX(seq) FROM session_events WHERE session_id = ?")
        .await?;
    let mut rows = stmt.query(params![session_id]).await?;
    if let Some(r) = rows.next().await? {
        Ok(r.get::<Option<i64>>(0)?.unwrap_or(0))
    } else {
        Ok(0)
    }
}

pub async fn after(
    conn: &Connection,
    session_id: &str,
    after_seq: i64,
) -> Result<Vec<SessionEventRecord>> {
    let stmt = conn
        .prepare("SELECT seq, type, payload_json, sse_kind, ts FROM session_events WHERE session_id = ? AND seq > ? ORDER BY seq ASC")
        .await?;
    let mut rows = stmt.query(params![session_id, after_seq]).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        let seq: i64 = r.get(0)?;
        let kind_s: String = r.get(1)?;
        let payload_json: String = r.get(2)?;
        let sse_kind: Option<String> = r.get(3)?;
        let ts: i64 = r.get(4)?;
        let payload: serde_json::Value = serde_json::from_str(&payload_json).unwrap_or_else(|e| {
            tracing::warn!(session_id, seq, error = %e, "failed to deserialize event payload, using null");
            serde_json::Value::Null
        });
        out.push(SessionEventRecord {
            session_id: session_id.to_string(),
            kind: parse_kind(&kind_s),
            payload,
            ts,
            seq: Some(seq),
            sse_kind,
        });
    }
    Ok(out)
}

pub async fn page(
    conn: &Connection,
    session_id: &str,
    after_seq: i64,
    limit: u32,
    payload_budget: usize,
) -> Result<SessionEventPage> {
    let limit = limit.clamp(1, 200) as usize;
    let mut rows = conn
        .query(
            "SELECT seq,type,sse_kind,ts,length(CAST(payload_json AS BLOB)) \
             FROM session_events WHERE session_id=?1 AND seq>?2 ORDER BY seq ASC LIMIT ?3",
            params![session_id, after_seq, limit as i64 + 1],
        )
        .await?;
    let mut metas = Vec::with_capacity(limit + 1);
    while let Some(row) = rows.next().await? {
        metas.push((
            row.get::<i64>(0)?,
            row.get::<String>(1)?,
            row.get::<Option<String>>(2)?,
            row.get::<i64>(3)?,
            row.get::<i64>(4)?.max(0) as usize,
        ));
    }
    drop(rows);

    let mut events = Vec::with_capacity(limit.min(metas.len()));
    let mut used = 0usize;
    for (seq, kind, sse_kind, ts, payload_len) in metas.iter().take(limit) {
        let row_bytes = payload_len.saturating_add(EVENT_OVERHEAD_BYTES);
        if row_bytes > payload_budget && events.is_empty() {
            events.push(SessionEventRecord {
                session_id: session_id.to_owned(),
                kind: parse_kind(kind),
                payload: serde_json::json!({
                    "omitted": true,
                    "reason": "event_payload_exceeds_page_budget",
                    "total_bytes": payload_len,
                    "read_via": "event_payload",
                }),
                ts: *ts,
                seq: Some(*seq),
                sse_kind: sse_kind.clone(),
            });
            break;
        }
        if used.saturating_add(row_bytes) > payload_budget {
            break;
        }
        let mut payload_rows = conn
            .query(
                "SELECT payload_json FROM session_events WHERE session_id=?1 AND seq=?2",
                params![session_id, *seq],
            )
            .await?;
        let Some(row) = payload_rows.next().await? else {
            anyhow::bail!("event {seq} disappeared during pagination");
        };
        let payload_json: String = row.get(0)?;
        let payload = serde_json::from_str(&payload_json).unwrap_or_else(|error| {
            tracing::warn!(session_id, seq, %error, "failed to deserialize event payload, using null");
            serde_json::Value::Null
        });
        events.push(SessionEventRecord {
            session_id: session_id.to_owned(),
            kind: parse_kind(kind),
            payload,
            ts: *ts,
            seq: Some(*seq),
            sse_kind: sse_kind.clone(),
        });
        used += row_bytes;
    }
    let more = events.len() < metas.len();
    Ok(SessionEventPage { events, more })
}

pub async fn payload_chunk(
    conn: &Connection,
    session_id: &str,
    seq: i64,
    offset: u64,
    max_bytes: usize,
) -> Result<Option<crate::PayloadChunkRecord>> {
    let take = max_bytes.clamp(1, 64 * 1024) as i64;
    let start = i64::try_from(offset)?.saturating_add(1);
    let mut rows = conn
        .query(
            "SELECT length(CAST(payload_json AS BLOB)), \
             CAST(substr(CAST(payload_json AS BLOB),?3,?4) AS BLOB) \
             FROM session_events WHERE session_id=?1 AND seq=?2",
            params![session_id, seq, start, take],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let total = row.get::<i64>(0)?.max(0) as u64;
    if offset > total {
        anyhow::bail!("event payload offset exceeds total bytes");
    }
    Ok(Some(crate::PayloadChunkRecord {
        total_bytes: total,
        bytes: row.get(1)?,
    }))
}

fn kind_str(k: EventKind) -> &'static str {
    match k {
        EventKind::PromptAdmitted => "prompt_admitted",
        EventKind::PromptPromoted => "prompt_promoted",
        EventKind::TextDelta => "text_delta",
        EventKind::ToolStart => "tool_start",
        EventKind::ToolEnd => "tool_end",
        EventKind::AgentSwitched => "agent_switched",
        EventKind::ModelSwitched => "model_switched",
        EventKind::Compaction => "compaction",
        EventKind::Step => "step",
        EventKind::Interrupted => "interrupted",
        EventKind::Done => "done",
        EventKind::Error => "error",
    }
}

fn parse_kind(s: &str) -> EventKind {
    match s {
        "prompt_admitted" => EventKind::PromptAdmitted,
        "prompt_promoted" => EventKind::PromptPromoted,
        "text_delta" => EventKind::TextDelta,
        "tool_start" => EventKind::ToolStart,
        "tool_end" => EventKind::ToolEnd,
        "agent_switched" => EventKind::AgentSwitched,
        "model_switched" => EventKind::ModelSwitched,
        "compaction" => EventKind::Compaction,
        "step" => EventKind::Step,
        "interrupted" => EventKind::Interrupted,
        "done" => EventKind::Done,
        "error" => EventKind::Error,
        _ => EventKind::Step,
    }
}
