//! Strict decoding of legacy SQLite rows.
//!
//! Every cell is decoded exactly as recorded or the row fails the migration:
//! unknown enum strings, malformed JSON columns, floating-point timestamps
//! and non-0/1 flags are hard errors. Rows are never coerced, defaulted or
//! silently dropped (the legacy runtime's read path was tolerant; a migration
//! must not be).

use anyhow::{anyhow, bail, Result};
use serde::de::DeserializeOwned;

use opencoder_core::{ContentBlock, Message, MessageUsage, Role};
use opencoder_store::{Delivery, EventKind, SubagentStatus};

use super::rows::{LegacyEvent, LegacyInput, LegacyMessage, LegacySession, LegacyTask};

/// Column order the extraction SELECTs pin down; a mismatch is a programming
/// error between `extract.rs` and the parsers, so it fails loudly.
pub(crate) fn require_columns(found: &[String], expected: &[&str]) -> Result<()> {
    let found_cols: Vec<&str> = found.iter().map(String::as_str).collect();
    if found_cols == expected {
        return Ok(());
    }
    bail!(
        "unexpected legacy column layout: expected [{}], got [{}]",
        expected.join(", "),
        found_cols.join(", ")
    );
}

fn cell<'a>(row: &'a [serde_json::Value], idx: usize, col: &str) -> Result<&'a serde_json::Value> {
    row.get(idx)
        .ok_or_else(|| anyhow!("column `{col}` missing from legacy row"))
}

fn str_cell<'a>(row: &'a [serde_json::Value], idx: usize, col: &str) -> Result<&'a str> {
    cell(row, idx, col)?
        .as_str()
        .ok_or_else(|| anyhow!("column `{col}` must be TEXT, got a non-text value"))
}

fn opt_str_cell(row: &[serde_json::Value], idx: usize, col: &str) -> Result<Option<String>> {
    match cell(row, idx, col)? {
        serde_json::Value::Null => Ok(None),
        v => Ok(Some(
            v.as_str()
                .ok_or_else(|| anyhow!("column `{col}` must be TEXT or NULL"))?
                .to_string(),
        )),
    }
}

fn i64_cell(row: &[serde_json::Value], idx: usize, col: &str) -> Result<i64> {
    let v = cell(row, idx, col)?;
    v.as_i64()
        .ok_or_else(|| anyhow!("column `{col}` must be an INTEGER, got {v}"))
}

fn opt_i64_cell(row: &[serde_json::Value], idx: usize, col: &str) -> Result<Option<i64>> {
    match cell(row, idx, col)? {
        serde_json::Value::Null => Ok(None),
        v => Ok(Some(v.as_i64().ok_or_else(|| {
            anyhow!("column `{col}` must be INTEGER or NULL, got {v}")
        })?)),
    }
}

/// A NOT NULL 0/1 flag; anything else (including NULL) is corrupt.
fn flag_cell(row: &[serde_json::Value], idx: usize, col: &str) -> Result<bool> {
    match opt_i64_cell(row, idx, col)? {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        other => bail!("column `{col}` must be 0 or 1, got {other:?}"),
    }
}

/// Strict JSON column: TEXT holding a JSON document of exactly type `T`.
fn json_cell<T: DeserializeOwned>(row: &[serde_json::Value], idx: usize, col: &str) -> Result<T> {
    let raw = str_cell(row, idx, col)?;
    serde_json::from_str(raw).map_err(|e| anyhow!("column `{col}` is not valid JSON: {e}"))
}

fn opt_json_cell<T: DeserializeOwned>(
    row: &[serde_json::Value],
    idx: usize,
    col: &str,
) -> Result<Option<T>> {
    match opt_str_cell(row, idx, col)? {
        None => Ok(None),
        Some(raw) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(|e| anyhow!("column `{col}` is not valid JSON: {e}")),
    }
}

/// A nullable 0/1 flag; any other integer is corrupt.
fn opt_flag_cell(row: &[serde_json::Value], idx: usize, col: &str) -> Result<Option<bool>> {
    match opt_i64_cell(row, idx, col)? {
        None => Ok(None),
        Some(0) => Ok(Some(false)),
        Some(1) => Ok(Some(true)),
        other => bail!("column `{col}` must be 0 or 1, got {other:?}"),
    }
}

/// Legacy enum strings — exact matches only, mirroring the writers' `as_str`
/// tables. Unknown values error instead of falling back.
pub(crate) fn parse_role(s: &str) -> Result<Role> {
    match s {
        "system" => Ok(Role::System),
        "user" => Ok(Role::User),
        "assistant" => Ok(Role::Assistant),
        "tool" => Ok(Role::Tool),
        _ => bail!("unknown legacy message role `{s}`"),
    }
}

pub(crate) fn parse_event_kind(s: &str) -> Result<EventKind> {
    match s {
        "prompt_admitted" => Ok(EventKind::PromptAdmitted),
        "prompt_promoted" => Ok(EventKind::PromptPromoted),
        "text_delta" => Ok(EventKind::TextDelta),
        "tool_start" => Ok(EventKind::ToolStart),
        "tool_end" => Ok(EventKind::ToolEnd),
        "agent_switched" => Ok(EventKind::AgentSwitched),
        "model_switched" => Ok(EventKind::ModelSwitched),
        "compaction" => Ok(EventKind::Compaction),
        "step" => Ok(EventKind::Step),
        "interrupted" => Ok(EventKind::Interrupted),
        "done" => Ok(EventKind::Done),
        "error" => Ok(EventKind::Error),
        _ => bail!("unknown legacy session_events type `{s}`"),
    }
}

pub(crate) fn parse_delivery(s: &str) -> Result<Delivery> {
    match s {
        "steer" => Ok(Delivery::Steer),
        "queue" => Ok(Delivery::Queue),
        _ => bail!("unknown legacy session_inputs delivery `{s}`"),
    }
}

pub(crate) fn parse_subagent_status(s: &str) -> Result<SubagentStatus> {
    match s {
        "running" => Ok(SubagentStatus::Running),
        "completed" => Ok(SubagentStatus::Completed),
        "failed" => Ok(SubagentStatus::Failed),
        "cancelled" => Ok(SubagentStatus::Cancelled),
        "unknown" => Ok(SubagentStatus::Unknown),
        _ => bail!("unknown legacy subagent_tasks status `{s}`"),
    }
}

/// `sessions` row (SELECT order pinned by `extract::SESSION_COLUMNS`:
/// id, kind, title, agent, model, autopilot_mode, workdir_hash, created_at,
/// updated_at, summary, summary_seq, summary_images_json, handoff_seq,
/// handoff_plan, skill, task_type, requirement).
pub(crate) fn parse_session(row: &[serde_json::Value], id: &str) -> Result<LegacySession> {
    let summary_images = opt_json_cell(row, 11, "summary_images_json")?.unwrap_or_default();
    Ok(LegacySession {
        meta: opencoder_store::SessionMeta {
            id: id.to_string(),
            kind: opt_str_cell(row, 1, "kind")?,
            title: opt_str_cell(row, 2, "title")?,
            agent: opt_str_cell(row, 3, "agent")?,
            model: opt_str_cell(row, 4, "model")?,
            autopilot_mode: opt_str_cell(row, 5, "autopilot_mode")?,
            workdir_hash: opt_str_cell(row, 6, "workdir_hash")?,
            created_at: i64_cell(row, 7, "created_at")?,
            updated_at: i64_cell(row, 8, "updated_at")?,
            summary: opt_str_cell(row, 9, "summary")?,
            summary_seq: opt_i64_cell(row, 10, "summary_seq")?,
            summary_images,
            handoff_seq: opt_i64_cell(row, 12, "handoff_seq")?,
            handoff_plan: opt_str_cell(row, 13, "handoff_plan")?,
            skill: opt_str_cell(row, 14, "skill")?,
            task_type: opt_str_cell(row, 15, "task_type")?,
            requirement: opt_str_cell(row, 16, "requirement")?,
        },
    })
}

/// `messages` row (SELECT order pinned by `extract::MESSAGE_COLUMNS`:
/// session_id, id, role, agent, model, blocks_json, provider_state_json,
/// usage_json, created_at, synthetic, display).
pub(crate) fn parse_message(row: &[serde_json::Value]) -> Result<LegacyMessage> {
    let session_id = str_cell(row, 0, "session_id")?.to_string();
    let id = str_cell(row, 1, "id")?.to_string();
    let role = parse_role(str_cell(row, 2, "role")?)?;
    let message = Message {
        provider_state: opt_json_cell(row, 6, "provider_state_json")?,
        id,
        role,
        blocks: json_cell::<Vec<ContentBlock>>(row, 5, "blocks_json")?,
        model: opt_str_cell(row, 4, "model")?,
        agent: opt_str_cell(row, 3, "agent")?,
        usage: json_cell::<MessageUsage>(row, 7, "usage_json")?,
        created_at: i64_cell(row, 8, "created_at")?,
        synthetic: flag_cell(row, 9, "synthetic")?,
        display: opt_str_cell(row, 10, "display")?,
    };
    Ok(LegacyMessage {
        session_id,
        message,
    })
}

/// `session_events` row (SELECT order pinned by `extract::EVENT_COLUMNS`).
pub(crate) fn parse_event(row: &[serde_json::Value]) -> Result<LegacyEvent> {
    Ok(LegacyEvent {
        record: opencoder_store::SessionEventRecord {
            session_id: str_cell(row, 0, "session_id")?.to_string(),
            kind: parse_event_kind(str_cell(row, 1, "type")?)?,
            payload: json_cell(row, 2, "payload_json")?,
            sse_kind: opt_str_cell(row, 3, "sse_kind")?,
            ts: i64_cell(row, 4, "ts")?,
            seq: None,
        },
    })
}

/// `session_inputs` row (SELECT order pinned by `extract::INPUT_COLUMNS`).
pub(crate) fn parse_input(row: &[serde_json::Value]) -> Result<LegacyInput> {
    Ok(LegacyInput {
        input: opencoder_store::SessionInput {
            seq: None,
            id: str_cell(row, 1, "id")?.to_string(),
            session_id: str_cell(row, 0, "session_id")?.to_string(),
            delivery: parse_delivery(str_cell(row, 2, "delivery")?)?,
            prompt: str_cell(row, 3, "prompt")?.to_string(),
            images: json_cell(row, 4, "images_json")?,
            display_text: opt_str_cell(row, 5, "display_text")?,
            admitted_seq: i64_cell(row, 6, "admitted_seq")?,
            promoted_seq: opt_i64_cell(row, 7, "promoted_seq")?,
        },
    })
}

/// `subagent_tasks` row (SELECT order pinned by `extract::TASK_COLUMNS`).
pub(crate) fn parse_subagent_task(row: &[serde_json::Value]) -> Result<LegacyTask> {
    Ok(LegacyTask {
        task: opencoder_store::SubagentTaskRecord {
            task_id: str_cell(row, 0, "task_id")?.to_string(),
            parent_session_id: str_cell(row, 1, "parent_session_id")?.to_string(),
            child_session_id: str_cell(row, 2, "child_session_id")?.to_string(),
            parent_message_id: opt_str_cell(row, 3, "parent_message_id")?,
            agent: str_cell(row, 4, "agent")?.to_string(),
            prompt: str_cell(row, 5, "prompt")?.to_string(),
            result: opt_str_cell(row, 6, "result")?,
            status: parse_subagent_status(str_cell(row, 7, "status")?)?,
            ok: opt_flag_cell(row, 8, "ok")?,
            started_at: i64_cell(row, 9, "started_at")?,
            completed_at: opt_i64_cell(row, 10, "completed_at")?,
        },
    })
}
