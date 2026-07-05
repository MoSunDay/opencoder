use anyhow::{Context, Result};
use libsql::{params, Connection};

use crate::types::{EventKind, SessionEventRecord};

const INSERT_EVENT: &str = "\
INSERT INTO session_events (session_id, type, payload_json, ts)
VALUES (?, ?, ?, ?)";

pub async fn append(conn: &Connection, event: &SessionEventRecord) -> Result<i64> {
    let payload_json = serde_json::to_string(&event.payload).context("serialize event payload")?;
    conn.execute(
        INSERT_EVENT,
        params![event.session_id.as_str(), kind_str(event.kind), payload_json, event.ts],
    )
    .await
    .context("insert event")?;
    let stmt = conn.prepare("SELECT MAX(seq) FROM session_events WHERE session_id = ?").await?;
    let mut rows = stmt.query(params![event.session_id.as_str()]).await?;
    if let Some(r) = rows.next().await? {
        Ok(r.get::<Option<i64>>(0)?.unwrap_or(0))
    } else {
        Ok(0)
    }
}

pub async fn after(conn: &Connection, session_id: &str, after_seq: i64) -> Result<Vec<SessionEventRecord>> {
    let stmt = conn
        .prepare("SELECT seq, type, payload_json, ts FROM session_events WHERE session_id = ? AND seq > ? ORDER BY seq ASC")
        .await?;
    let mut rows = stmt.query(params![session_id, after_seq]).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        let seq: i64 = r.get(0)?;
        let kind_s: String = r.get(1)?;
        let payload_json: String = r.get(2)?;
        let ts: i64 = r.get(3)?;
        let payload: serde_json::Value = serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null);
        out.push(SessionEventRecord {
            session_id: session_id.to_string(),
            kind: parse_kind(&kind_s),
            payload,
            ts,
            seq: Some(seq),
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
