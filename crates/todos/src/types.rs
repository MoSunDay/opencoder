use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

fn default_agent() -> String {
    "act".into()
}

fn default_attempts() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowSpec {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub objective: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    pub todos: Vec<TodoSpec>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TodoSpec {
    pub id: String,
    pub title: String,
    pub requirement_background: String,
    pub instructions: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default = "default_agent")]
    pub agent: String,
    #[serde(default = "default_attempts")]
    pub max_attempts: u32,
    pub acceptance: AcceptanceSpec,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AcceptanceSpec {
    pub criteria: String,
    #[serde(default)]
    pub required_tool_calls: Vec<RequiredToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequiredToolCall {
    pub name: String,
    #[serde(default)]
    pub arguments_contains: serde_json::Value,
    #[serde(default = "default_true")]
    pub result_ok: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Pending,
    Running,
    Suspended,
    Completed,
    Failed,
}

impl WorkflowStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Suspended => "suspended",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    Running,
    CandidateReady,
    Accepting,
    NeedsRevision,
    Passed,
    Interrupted,
    Invalidated,
    Recovering,
    Failed,
}

impl TodoStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::CandidateReady => "candidate_ready",
            Self::Accepting => "accepting",
            Self::NeedsRevision => "needs_revision",
            Self::Passed => "passed",
            Self::Interrupted => "interrupted",
            Self::Invalidated => "invalidated",
            Self::Recovering => "recovering",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candidate {
    pub status: CandidateStatus,
    pub summary: String,
    pub result: Option<String>,
    pub verification: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recovery_context: RecoveryContext,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    Candidate,
    Blocked,
    Interrupted,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryContext {
    pub summary: String,
    #[serde(default)]
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoState {
    pub status: TodoStatus,
    pub attempt: u32,
    pub active_session_id: Option<String>,
    pub session_history: Vec<String>,
    pub candidate: Option<Candidate>,
    pub last_error: Option<String>,
    pub accepted_generation: Option<u64>,
    pub next_context_mode: Option<ContextMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowState {
    pub workflow_id: String,
    pub parent_session_id: String,
    pub status: WorkflowStatus,
    pub generation: u64,
    pub world_epoch: u64,
    pub active_todo_ids: BTreeSet<String>,
    pub todos: BTreeMap<String, TodoState>,
    #[serde(default)]
    pub milestones: BTreeSet<String>,
    #[serde(default)]
    pub incidents: Vec<serde_json::Value>,
    pub terminal_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum ParentDecision {
    Dispatch {
        todos: Vec<DispatchTodo>,
        reason: String,
    },
    MarkMilestone {
        todo_id: String,
        reason: String,
    },
    Rewind {
        milestone_todo_id: String,
        reason: String,
    },
    Complete {
        reason: String,
    },
    Fail {
        reason: String,
    },
    Suspend {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchTodo {
    pub todo_id: String,
    pub context_mode: ContextMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    New,
    Resume,
    Fork,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum AcceptanceDecision {
    Accept {
        reason: String,
        mark_milestone: bool,
    },
    Revise {
        reason: String,
        context_mode: ContextMode,
    },
    Fail {
        reason: String,
    },
    Rewind {
        milestone_todo_id: String,
        reason: String,
    },
}
