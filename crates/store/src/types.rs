use opencoder_core::Message;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir_hash: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_seq: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct SessionFilter {
    pub limit: u32,
    pub cursor: Option<String>,
    pub workdir_hash: Option<String>,
    pub search: Option<String>,
    pub include_subagents: bool,
}

impl Default for SessionFilter {
    fn default() -> Self {
        SessionFilter {
            limit: 50,
            cursor: None,
            workdir_hash: None,
            search: None,
            include_subagents: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionListItem {
    pub id: String,
    pub title: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub preview: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Delivery {
    #[default]
    Steer,
    Queue,
}

impl Delivery {
    pub fn as_str(&self) -> &'static str {
        match self {
            Delivery::Steer => "steer",
            Delivery::Queue => "queue",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "steer" => Some(Delivery::Steer),
            "queue" => Some(Delivery::Queue),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInput {
    pub id: String,
    pub session_id: String,
    pub delivery: Delivery,
    pub prompt: String,
    pub admitted_seq: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_seq: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    PromptAdmitted,
    PromptPromoted,
    TextDelta,
    ToolStart,
    ToolEnd,
    AgentSwitched,
    ModelSwitched,
    Compaction,
    Step,
    Interrupted,
    Done,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEventRecord {
    pub session_id: String,
    pub kind: EventKind,
    pub payload: serde_json::Value,
    pub ts: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    pub sessions: u32,
    pub messages: u32,
    pub skipped: u32,
}

/// Lifecycle status of a subagent task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Running,
    Completed,
    Failed,
}

impl SubagentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubagentStatus::Running => "running",
            SubagentStatus::Completed => "completed",
            SubagentStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "completed" => SubagentStatus::Completed,
            "failed" => SubagentStatus::Failed,
            _ => SubagentStatus::Running,
        }
    }
}

/// A parent-child agent relationship record stored in `subagent_tasks`.
/// Captures the prompt submitted, the final result, and lifecycle metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentTaskRecord {
    pub task_id: String,
    pub parent_session_id: String,
    pub child_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
    pub agent: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    pub status: SubagentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
}

pub fn message_preview(msgs: &[Message], max_chars: usize) -> String {
    let mut out = String::new();
    for m in msgs {
        if m.role != opencoder_core::Role::User {
            continue;
        }
        let t = m.text();
        if t.is_empty() {
            continue;
        }
        out = t.chars().take(max_chars).collect();
        break;
    }
    out
}
