use super::{AccessMode, OntologyPlan, PlanRef, PlanVersion};
use crate::fleet::{ExecutionKind, ExecutionRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BrainRequest {
    pub mode: PlanningMode,
    pub objective: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub plan: Option<PlanVersion>,
    #[serde(default)]
    pub references: Vec<PlanVersion>,
    #[serde(default)]
    pub capabilities: Vec<Value>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanningMode {
    Fixed,
    Dynamic,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    Planning,
    Running,
    Paused,
    WaitingInput,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}

impl RunPhase {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BrainRun {
    pub id: String,
    pub phase: RunPhase,
    pub revision: u64,
    pub control_epoch: u64,
    pub activation: u64,
    pub handled_revision: u64,
    pub request: BrainRequest,
    #[serde(default)]
    pub candidate_plan: Option<PlanVersion>,
    pub instances: BTreeMap<String, StepInstance>,
    /// A sealed entry is the complete finite instance set for a template.
    pub expansions: BTreeMap<String, Vec<String>>,
    pub source_cursors: BTreeMap<String, u64>,
    pub actions: BTreeMap<String, ActionReceipt>,
    pub input_requests: BTreeMap<String, InputRequest>,
    pub deliverables: BTreeMap<String, Value>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StepInstance {
    pub id: String,
    pub step_id: String,
    pub item_key: Option<String>,
    pub item: Value,
    pub status: StepStatus,
    pub attempt: u32,
    pub execution: Option<ExecutionRef>,
    pub node_id: Option<String>,
    pub inputs: BTreeMap<String, Value>,
    pub output: Option<OutputEnvelope>,
    pub reason: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Waiting,
    Ready,
    Queued,
    Running,
    Verifying,
    Succeeded,
    Failed,
    Skipped,
    Cancelled,
}

impl StepStatus {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Skipped | Self::Cancelled
        )
    }
    pub fn active(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Verifying)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OutputEnvelope {
    pub value: Value,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRef {
    pub execution: ExecutionRef,
    pub step: String,
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ActionReceipt {
    pub id: String,
    pub instance_id: String,
    pub attempt: u32,
    pub kind: ActionKind,
    pub state: ReceiptState,
    pub input_fingerprint: String,
    pub request: Value,
    pub result: Option<Value>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Execute,
    Cancel,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptState {
    Prepared,
    Accepted,
    Settled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InputRequest {
    pub name: String,
    pub description: String,
    pub schema: super::DataSchema,
    pub answered: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrainLink {
    pub run_id: String,
    pub node_id: String,
    pub instance_id: String,
    pub attempt: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BrainNotice {
    pub parent: BrainLink,
    pub execution: ExecutionRef,
    pub node_id: String,
    pub sequence: u64,
    pub status: StepStatus,
    pub output: Option<OutputEnvelope>,
    pub error: Option<String>,
    pub at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivationContext {
    pub schema_version: u32,
    pub run_id: String,
    pub activation: u64,
    pub control_epoch: u64,
    pub revision: u64,
    pub plan_ref: Option<PlanRef>,
    pub plan: Option<OntologyPlan>,
    pub objective: String,
    pub phase: RunPhase,
    pub inputs: BTreeMap<String, Value>,
    pub instances: Vec<StepInstance>,
    pub ready: Vec<String>,
    pub references: Vec<PlanVersion>,
    pub capabilities: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivationDecision {
    pub run_id: String,
    pub activation: u64,
    pub control_epoch: u64,
    pub reason: String,
    #[serde(default)]
    pub plan: Option<OntologyPlan>,
    #[serde(default)]
    pub dispatch: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceClaim {
    pub resource: String,
    pub mode: AccessMode,
    pub execution_id: String,
    pub run_id: String,
    pub released: bool,
}

pub const BUSINESS_KINDS: [ExecutionKind; 4] = [
    ExecutionKind::Agent,
    ExecutionKind::Dag,
    ExecutionKind::Todos,
    ExecutionKind::Team,
];
