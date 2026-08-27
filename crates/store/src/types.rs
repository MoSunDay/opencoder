use opencoder_core::Message;
use serde::{Deserialize, Serialize};

/// Normal top-level session.
pub const TASK_TYPE_PARENT: &str = "parent";
/// Child session spawned by a `task` subagent invocation.
pub const TASK_TYPE_SUBAGENT: &str = "subagent";
/// Internal parent session used by the todos workflow scheduler.
pub const TASK_TYPE_TODO_WORKFLOW: &str = "todo_workflow";
/// Full primary session assigned to one focused TODO.
pub const TASK_TYPE_TODO: &str = "todo";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Session-scoped autopilot mode for the `/ap` "session-only" switch.
    /// `None` = follow the global config; `Some("off"|"ap"|"review")` pins
    /// this session's mode so resume honors it (same role as `model`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autopilot_mode: Option<String>,
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
    /// Image URLs preserved across compaction (most recent <=4), persisted
    /// to `summary_images_json` so resume can rebuild the synthetic summary
    /// message WITHOUT reloading the soft-deleted compacted head.
    #[serde(default)]
    pub summary_images: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    /// Lifecycle role of this session: `"parent"` for normal top-level
    /// sessions, `"subagent"` for child sessions spawned by a `task` subagent.
    /// `None` (in-memory default) is treated as `"parent"`. Stored as a
    /// NOT NULL column in the DB so it can be indexed and filtered cheaply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    /// User-edited task description text, persisted via the /requirement
    /// slash command so it survives session resume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    /// Pre-compaction snapshot of the finalized plan text for plan-mode
    /// sessions. Captured by compaction before the plan assistant message
    /// can be folded into the summary head, so a later plan->act handoff
    /// still finds the plan even when it slid out of the retained tail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_snapshot: Option<String>,
    /// Number of user prompts recorded in the current plan phase (since the
    /// last handoff or re-entry into plan mode). Persisted so a resumed
    /// session can re-arm plan-phase affordances (TUI Shift+Tab handoff,
    /// /act_clear_context plan-provenance gate) that were previously lost
    /// on restart.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub plan_input_count: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Sets the session-scoped autopilot mode; see `SessionMeta::autopilot_mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autopilot_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_images: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_plan: Option<String>,
    /// When true, clears all compaction metadata: `summary`, `summary_seq`,
    /// and `summary_images_json` are set to NULL. Used by the handoff path to
    /// remove stale compaction state -- handoff and compaction are mutually
    /// exclusive, and handoff supersedes any prior compaction. Without this,
    /// a residual `summary_seq` would be picked over the newer `handoff_seq`
    /// in `prev_skip = summary_seq.or(handoff_seq)`, producing an OFFSET that
    /// is too small and re-loading already-summarized messages.
    #[serde(default, skip_serializing_if = "is_false")]
    pub clear_summary: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub clear_handoff: bool,
    /// When true, sets `skill` to NULL (clears the active skill). Used
    /// separately from `skill: None` (which means "don't touch").
    #[serde(default, skip_serializing_if = "is_false")]
    pub clear_skill: bool,
    /// When true, sets `agent` to NULL (clears the persisted agent). Used
    /// separately from `agent: None` (which means "don't touch"): the web
    /// layer's TOCTOU rollback needs it to restore a NULL agent — a plain
    /// `agent: None` patch would be a silent no-op.
    #[serde(default, skip_serializing_if = "is_false")]
    pub clear_agent: bool,
    /// When true, sets `model` to NULL (clears the persisted model). Same
    /// purpose as `clear_agent` for the model column.
    #[serde(default, skip_serializing_if = "is_false")]
    pub clear_model: bool,
    /// When true, sets `autopilot_mode` to NULL (back to "follow the global
    /// config"). Same purpose as `clear_model` for the autopilot_mode column.
    #[serde(default, skip_serializing_if = "is_false")]
    pub clear_autopilot_mode: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    /// When true, sets `requirement` to NULL (clears the user annotation).
    /// Used separately from `requirement: None` (which means "don't touch") so
    /// an explicit empty annotation save can be distinguished from no-op.
    #[serde(default, skip_serializing_if = "is_false")]
    pub clear_requirement: bool,
    /// Mirrors the in-memory plan snapshot onto the sessions row. Compaction
    /// writes it while the plan text is still extractable; see
    /// `SessionMeta::plan_snapshot`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_snapshot: Option<String>,
    /// When true, sets `plan_snapshot` to NULL (consumed by a plan->act
    /// handoff or reset by plan-phase re-entry). Mutually exclusive with the
    /// `plan_snapshot` value.
    #[serde(default, skip_serializing_if = "is_false")]
    pub clear_plan_snapshot: bool,
    /// Persists the plan-phase input counter; see
    /// `SessionMeta::plan_input_count`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_input_count: Option<i64>,
}

fn is_false(b: &bool) -> bool {
    !b
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

impl SessionPatch {
    /// Build the rollback patch for the `agent` column from the pre-switch
    /// row captured by the caller.
    ///
    /// Restores the captured value when present. When the column was NULL —
    /// or the capture read failed (`old` is `None`; a drain running implies
    /// the row exists) — the column is CLEARED: `agent: None` alone means
    /// "don't touch", so it would be a silent no-op and leave the refused
    /// switch persisted. Always bumps `updated_at`.
    pub fn rollback_agent(old: Option<&SessionMeta>) -> SessionPatch {
        let v = old.and_then(|m| m.agent.clone());
        SessionPatch {
            agent: v.clone(),
            clear_agent: v.is_none(),
            updated_at: Some(opencoder_core::message::now_ms()),
            ..Default::default()
        }
    }

    /// Build the rollback patch for the `model` column from the pre-switch
    /// row. See [`SessionPatch::rollback_agent`] for the NULL / failed-read
    /// clearing semantics.
    pub fn rollback_model(old: Option<&SessionMeta>) -> SessionPatch {
        let v = old.and_then(|m| m.model.clone());
        SessionPatch {
            model: v.clone(),
            clear_model: v.is_none(),
            updated_at: Some(opencoder_core::message::now_ms()),
            ..Default::default()
        }
    }
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
    /// The session's active skill **body** (full instruction text), when one
    /// is set. The store persists the body, not the name; display layers
    /// derive a name by matching it against `discover_skills()`.
    #[serde(default)]
    pub skill: Option<String>,
    pub model: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub preview: String,
    /// Number of subagent tasks still in-flight (`Running`) for this session,
    /// derived from `subagent_tasks` at list time. `0` when none.
    #[serde(default)]
    pub subagent_running: usize,
    /// Number of subagent tasks interrupted (`Cancelled`, pending replay on the
    /// next user turn), derived from `subagent_tasks` at list time. `0` when
    /// none.
    #[serde(default)]
    pub subagent_cancelled: usize,
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
    /// Parse a delivery name, tolerating case and surrounding whitespace
    /// (`" queue "` is `Queue`). Returns `None` for unrecognized values.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "steer" => Some(Delivery::Steer),
            "queue" => Some(Delivery::Queue),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
    pub id: String,
    pub session_id: String,
    pub delivery: Delivery,
    pub prompt: String,
    /// Image attachments carried alongside the prompt as data URIs
    /// (`data:image/<fmt>;base64,...`). Empty for plain-text inputs. Persisted
    /// in `session_inputs.images_json` so steered/queued/resumed inputs keep
    /// their images across turn boundaries and restarts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// Display-only text for the TUI queue/steer panel, preserved verbatim
    /// (may contain the `$skill` token). NULL for rows admitted without a
    /// distinct display form — consumers fall back to `prompt`. Never fed to
    /// the LLM (drain always reads `prompt`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_text: Option<String>,
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
    /// Granular SSE event-name string preserved for lossless replay.
    /// Older records (pre-migration) lack this; callers fall back to
    /// `event_kind_str(kind)` when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sse_kind: Option<String>,
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
    /// Interrupted mid-run. The parent `task` tool_use is left open (no
    /// tool_result) so the child can be replayed on the next user turn. Distinct
    /// from `Failed` (a natural error result) and `Completed` (a real result).
    Cancelled,
    /// Forward-compat fallback produced only by `#[serde(other)]` when an
    /// unknown status string is deserialized from JSON. The DB TEXT path uses
    /// `as_str()`/`parse()`; `"unknown"` round-trips to `Unknown`, while any
    /// other unrecognized string still falls back to `Running` (still-in-flight).
    #[serde(other)]
    Unknown,
}

impl SubagentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubagentStatus::Running => "running",
            SubagentStatus::Completed => "completed",
            SubagentStatus::Failed => "failed",
            SubagentStatus::Cancelled => "cancelled",
            SubagentStatus::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "running" => SubagentStatus::Running,
            "completed" => SubagentStatus::Completed,
            "failed" => SubagentStatus::Failed,
            "cancelled" => SubagentStatus::Cancelled,
            "unknown" => SubagentStatus::Unknown,
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

/// Synthetic session created to execute one dispatched node task
/// (`node_tasks.session_id`; mirrors `TASK_TYPE_PARENT` / `TASK_TYPE_TODO`).
pub const TASK_TYPE_NODE: &str = "node";

/// A registered worker node in the multi-node fleet (`nodes` table row).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRecord {
    /// Server-issued ULID. Stable across re-registrations of the same
    /// `name`, so already-dispatched node tasks never dangle.
    pub id: String,
    /// User-friendly unique name (the upsert key for re-registration).
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    /// First registration time (epoch ms, server clock). Never rewritten.
    #[serde(default)]
    pub first_seen: i64,
    /// Last heartbeat time (epoch ms, server receive clock).
    #[serde(default)]
    pub last_seen_at: i64,
    /// Derived liveness: `online` | `idle` | `busy` | `lost`.
    #[serde(default)]
    pub last_status: String,
    /// Most recently claimed node task (kept after completion so UIs can show
    /// the latest work); see `node_tasks.id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_task_id: Option<String>,
    /// Last observed address: the client-declared value, else the TCP source
    /// IP captured at registration. None only for pre-migration rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_addr: Option<String>,
}

/// Lifecycle status of a dispatched node task.
///
/// State machine: `pending -> running -> done | error | cancelled`;
/// `pending/running -> cancelling -> cancelled | error | done` is also a legal
/// collapse. Terminal states (`done`/`error`/`cancelled`) never transition again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeTaskStatus {
    Pending,
    Running,
    Done,
    Error,
    Cancelled,
    /// A cancel was requested while the task was pending or running; the node
    /// picks it up on its next heartbeat and collapses to a terminal state.
    Cancelling,
}

impl NodeTaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeTaskStatus::Pending => "pending",
            NodeTaskStatus::Running => "running",
            NodeTaskStatus::Done => "done",
            NodeTaskStatus::Error => "error",
            NodeTaskStatus::Cancelled => "cancelled",
            NodeTaskStatus::Cancelling => "cancelling",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "running" => NodeTaskStatus::Running,
            "done" => NodeTaskStatus::Done,
            "error" => NodeTaskStatus::Error,
            "cancelled" => NodeTaskStatus::Cancelled,
            "cancelling" => NodeTaskStatus::Cancelling,
            _ => NodeTaskStatus::Pending,
        }
    }

    /// Terminal states accept no further transitions.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            NodeTaskStatus::Done | NodeTaskStatus::Error | NodeTaskStatus::Cancelled
        )
    }
}

/// One queued/executed task on a worker node (`node_tasks` table row).
/// Each task owns exactly one synthetic session (`task_type == "node"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTaskRecord {
    pub id: String,
    pub node_id: String,
    /// Synthetic session id driving the execution; unique per task.
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub status: NodeTaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Cancel flag set by `request_node_task_cancel`; read by the node's
    /// heartbeat alongside `status == cancelling`.
    #[serde(default)]
    pub cancel_requested: bool,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
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

/// Raw persisted message row — the read model of the P3 node message relay.
///
/// Unlike [`Message`] this keeps the per-session `seq` (the resume/compaction
/// boundary unit) and the blocks as an already-parsed raw JSON value, so a
/// relay can forward exactly what the worker stored without re-interpreting
/// block kinds. Produced by `Store::load_message_rows`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRow {
    /// Persisted per-session sequence number (`messages.seq`).
    pub seq: i64,
    /// Stored role literal: `system | user | assistant | tool`.
    pub role: String,
    /// Raw stored `blocks_json`, parsed as a JSON value (array of blocks).
    pub blocks: serde_json::Value,
    /// Emitter clock (epoch ms) persisted with the row.
    pub created_at: i64,
}
