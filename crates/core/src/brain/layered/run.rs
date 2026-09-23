//! Durable v4 projection: runs, layer operations (one per attempt) and events.
use crate::fleet::{ExecutionKind, ExecutionStatus};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayeredPhase {
    Ready,
    Deciding,
    Waiting,
    Paused,
    Blocked,
    Completed,
    Failed,
    Cancelled,
}
impl LayeredPhase {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayeredOperationStatus {
    Creating,
    Running,
    Done,
    Error,
    Cancelled,
}
impl LayeredOperationStatus {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Done | Self::Error | Self::Cancelled)
    }
    pub fn successful(self) -> bool {
        self == Self::Done
    }
    pub fn from_terminal(status: ExecutionStatus) -> Option<Self> {
        match status {
            ExecutionStatus::Done => Some(Self::Done),
            ExecutionStatus::Error => Some(Self::Error),
            ExecutionStatus::Cancelled => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredRun {
    #[serde(default = "first_round")]
    pub round: u32,
    #[serde(default)]
    pub activation: u64,
    #[serde(default)]
    pub valid_layers: u32,
    #[serde(default = "default_budget")]
    pub max_rounds: u32,
    #[serde(default)]
    pub reflection: Option<String>,
    pub run_id: String,
    pub phase: LayeredPhase,
    /// Completed layers; the layer being decided is always `layer + 1`.
    pub layer: u32,
    pub generation: u64,
    pub last_event_seq: u64,
    pub error: Option<String>,
    /// Final summary produced by the `complete` decision; a parent plan binds
    /// to it through the child's journal result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<crate::brain::layered::LayeredParent>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One node attempt: `operation_id` and `execution_id` both carry `attempt`,
/// so a late terminal from a superseded attempt can never be mistaken for the
/// live one.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredOperation {
    #[serde(default = "first_round")]
    pub round: u32,
    #[serde(default)]
    pub activation: u64,
    pub operation_id: String,
    pub run_id: String,
    pub layer: u32,
    pub node_id: String,
    pub attempt: u32,
    pub capability_id: String,
    pub execution_kind: ExecutionKind,
    pub execution_id: String,
    pub status: LayeredOperationStatus,
    pub source_sequence: Option<u64>,
    pub cancel_requested: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub assessments: std::collections::BTreeMap<String, super::MilestoneAssessment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assignments: Vec<super::LayeredAssignment>,
    #[serde(default = "first_round")]
    pub round: u32,
    #[serde(default)]
    pub activation: u64,
    pub seq: u64,
    pub run_id: String,
    pub layer: u32,
    pub event_type: String,
    pub node_id: Option<String>,
    pub attempt: Option<u32>,
    pub capability_id: Option<String>,
    pub execution_kind: Option<ExecutionKind>,
    pub execution_id: Option<String>,
    pub decision_summary: Option<String>,
    pub reason_summary: Option<String>,
    pub source_sequence: Option<u64>,
    pub evidence_execution_ids: Vec<String>,
    pub at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredTerminalEvent {
    pub run_id: String,
    pub operation_id: String,
    pub execution_kind: ExecutionKind,
    pub execution_id: String,
    pub status: LayeredOperationStatus,
    pub source_sequence: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredSnapshot {
    pub schema_version: u32,
    pub run: LayeredRun,
    pub operations: Vec<LayeredOperation>,
}

/// Atomic projection change; no execution input or output bodies.
#[derive(Clone, Debug)]
pub struct LayeredChange {
    pub expected_generation: Option<u64>,
    pub run: LayeredRun,
    pub operations: Vec<LayeredOperation>,
    pub events: Vec<LayeredEvent>,
}

/// Terminal payload written into the root journal: receipts plus the summary
/// contract the parent plan binds to.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredRunResult {
    pub schema_version: u32,
    pub phase: LayeredPhase,
    pub layer: u32,
    pub error: Option<String>,
    pub scheduler_output: serde_json::Value,
    pub scheduler_artifacts: serde_json::Value,
}

fn first_round() -> u32 {
    1
}
fn default_budget() -> u32 {
    5
}
