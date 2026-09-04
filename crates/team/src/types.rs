//! On-disk data model (all JSON, serde-default tolerant so a newer runtime
//! can still read an older file) + captain decision DTOs with a
//! fence/noise-tolerant parser (todos/json_output.rs style).

use anyhow::{anyhow, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

// ── team.json ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberRef {
    pub node_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub node_id: String,
    pub name: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub profiled_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMeta {
    pub name: String,
    pub captain: MemberRef,
    #[serde(default)]
    pub members: Vec<TeamMember>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

// ── topic team.json ────────────────────────────────────────────────────────

pub const TOPIC_EXECUTING: &str = "executing";
pub const TOPIC_FINISHED: &str = "finished";

pub const FINISH_COMPLETE: &str = "complete";
pub const FINISH_MAX_TURNS: &str = "max_turns";
pub const FINISH_MAX_SUB_TURNS: &str = "max_sub_turns";
pub const FINISH_CANCELLED: &str = "cancelled";
pub const FINISH_ERROR: &str = "error";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicMeta {
    pub topic_id: String,
    pub team_name: String,
    pub title: String,
    pub requirement: String,
    /// `executing` | `finished`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    pub captain: MemberRef,
    /// Membership snapshot taken at `start_topic` (the plan decision may only
    /// pick participants from here).
    #[serde(default)]
    pub members: Vec<MemberRef>,
    #[serde(default)]
    pub turns: Vec<TopicTurnMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicTurnMeta {
    pub turn: usize,
    pub question: String,
    #[serde(default)]
    pub participants: Vec<String>,
    #[serde(default)]
    pub aligned: bool,
    #[serde(default)]
    pub sub_turns: usize,
}

// ── per-turn / per-sub-turn records ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRecord {
    pub turn: usize,
    pub question: String,
    #[serde(default)]
    pub participants: Vec<String>,
    #[serde(default)]
    pub rationale: String,
}

pub const RESULT_ANSWER: &str = "answer";
pub const RESULT_ALIGNMENT: &str = "alignment";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultRecord {
    pub node_id: String,
    pub turn: usize,
    pub sub_turn: usize,
    /// `answer` | `alignment`.
    pub kind: String,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ambiguity {
    pub node_id: String,
    pub question: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryRecord {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub aligned: bool,
    #[serde(default)]
    pub ambiguities: Vec<Ambiguity>,
    #[serde(default)]
    pub created_at: i64,
}

// ── captain decision DTOs ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDecision {
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub participants: Vec<String>,
    #[serde(default)]
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryDecision {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub aligned: bool,
    #[serde(default)]
    pub ambiguities: Vec<Ambiguity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosingDecision {
    #[serde(default)]
    pub complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_question: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileDecision {
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// Parse a decision reply: a raw JSON object, one complete Markdown fence, or
/// a single object surrounded by prose noise. Multiple top-level fences or a
/// truncated fence are errors so the runtime re-asks instead of guessing.
pub fn parse_decision<T: DeserializeOwned>(raw: &str) -> Result<T> {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    // One complete ```json / ``` fence; prose may surround it. JSON strings
    // never contain raw newlines, so the first `\n``` ` after the opening
    // marker can only be the closing fence.
    if let Some(index) = trimmed.find("```") {
        let after = &trimmed[index..];
        let body = after
            .strip_prefix("```json")
            .or_else(|| after.strip_prefix("```"))
            .unwrap_or(after);
        let close = body
            .find("\n```")
            .ok_or_else(|| anyhow!("unterminated JSON fence"))?;
        if body[close + 4..].contains("```") {
            // Two fenced documents: ambiguous, refuse instead of guessing.
            return Err(anyhow!(
                "multiple JSON fences are not a structured response"
            ));
        }
        let inner = body[..close].trim();
        let value: T =
            serde_json::from_str(inner).context("fenced JSON does not match the required shape")?;
        return Ok(value);
    }
    // Prose around a single object: `说明 {"a":1} 补充` → the object.
    if let (Some(open), Some(close)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if open < close {
            if let Ok(value) = serde_json::from_str(&trimmed[open..=close]) {
                return Ok(value);
            }
        }
    }
    Err(anyhow!("reply is not a JSON object"))
}
