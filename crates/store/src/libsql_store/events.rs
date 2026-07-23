use anyhow::{Context, Result};
use libsql::{params, Connection};

use crate::types::{EventKind, SessionEventRecord};

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
            .prepare("SELECT seq FROM session_events WHERE session_id = ? ORDER BY seq DESC LIMIT ?")
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
        let payload: serde_json::Value =
            serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null);
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
