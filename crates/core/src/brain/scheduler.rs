//! V3 scheduling stores indexes and references; execution bodies stay on owners.
use crate::fleet::{ExecutionKind, ExecutionStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const SCHEDULER_SCHEMA_VERSION: u32 = 3;
pub const SCHEDULER_MIGRATION: &str =
    "migration required: new brain runs require explicit schema_version: 3; v2 runs are read-only";
fn max_rounds() -> u32 {
    32
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BrainSchedulerRequest {
    pub schema_version: u32,
    pub objective: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, Value>,
    #[serde(default = "max_rounds")]
    pub max_rounds: u32,
    #[serde(default)]
    pub capability_ids: Vec<String>,
    #[serde(default)]
    pub artifacts: BTreeMap<String, super::ArtifactRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BrainCapabilityDescriptor {
    pub capability_id: String,
    pub kind: ExecutionKind,
    pub target: String,
    pub input_desc: String,
    pub output_desc: String,
    #[serde(default)]
    pub required_inputs: Vec<String>,
    pub definition: Value,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrainInputBinding {
    Root { name: String },
    Execution { execution_id: String, path: String },
    Artifact { reference: String },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BrainDispatchItem {
    pub capability_id: String,
    pub inputs: BTreeMap<String, BrainInputBinding>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "decision", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrainSchedulerDecision {
    Dispatch {
        capabilities: Vec<BrainDispatchItem>,
        reason: String,
        evidence_execution_ids: Vec<String>,
    },
    Complete {
        reason: String,
        evidence_execution_ids: Vec<String>,
    },
    Fail {
        reason: String,
        error_type: String,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrainSchedulerPhase {
    Ready,
    Deciding,
    Waiting,
    Paused,
    Blocked,
    Completed,
    Failed,
    Cancelled,
}
impl BrainSchedulerPhase {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrainOperationStatus {
    Creating,
    Running,
    Done,
    Error,
    Cancelled,
}
impl BrainOperationStatus {
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
pub struct BrainSchedulerRun {
    pub run_id: String,
    pub phase: BrainSchedulerPhase,
    /// Zero before the first dispatch; dispatch increments the round.
    pub round: u32,
    pub generation: u64,
    pub last_event_seq: u64,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BrainOperation {
    pub operation_id: String,
    pub run_id: String,
    pub round: u32,
    pub capability_id: String,
    pub execution_kind: ExecutionKind,
    pub execution_id: String,
    pub status: BrainOperationStatus,
    pub source_sequence: Option<u64>,
    pub cancel_requested: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BrainSchedulerEvent {
    pub seq: u64,
    pub run_id: String,
    pub round: u32,
    pub event_type: String,
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
pub struct BrainSchedulerTerminalEvent {
    pub run_id: String,
    pub operation_id: String,
    pub execution_kind: ExecutionKind,
    pub execution_id: String,
    pub status: BrainOperationStatus,
    pub source_sequence: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BrainSchedulerSnapshot {
    pub schema_version: u32,
    pub run: BrainSchedulerRun,
    pub operations: Vec<BrainOperation>,
}
/// Atomic projection change; no execution input or output bodies.
#[derive(Clone, Debug)]
pub struct BrainSchedulerChange {
    pub expected_generation: Option<u64>,
    pub run: BrainSchedulerRun,
    pub operations: Vec<BrainOperation>,
    pub events: Vec<BrainSchedulerEvent>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrainSchedulerContext {
    pub schema_version: u32,
    pub run_id: String,
    pub generation: u64,
    pub round: u32,
    pub request: BrainSchedulerRequest,
    pub capabilities: Vec<BrainCapabilityDescriptor>,
    pub operations: Vec<BrainOperation>,
    /// Bounded necessary summaries fetched from the execution owners.
    pub summaries: BTreeMap<String, String>,
}
/// Root execution's durable dispatch intent. Contains only bindings and
/// registered definitions, never resolved input or child result bodies.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrainDispatchIntent {
    pub generation: u64,
    pub operations: Vec<BrainOperation>,
    pub items: Vec<BrainDispatchItem>,
    pub capabilities: Vec<BrainCapabilityDescriptor>,
}
