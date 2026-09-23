//! Milestone decisions and the bounded context handed to the model.
use super::{LayeredOperationStatus, LayeredPhase};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Inputs bind root values, generated tasks, artifacts, or terminal execution outputs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredAssignment {
    #[serde(default)]
    pub capability_id: String,
    pub node_id: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, crate::brain::BrainInputBinding>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "decision", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayeredDecision {
    Block {
        reason: String,
    },
    DispatchLayer {
        /// Next layer, or a configured reflection target.
        layer: u32,
        #[serde(default)]
        reflection: Option<String>,
        /// Every milestone in the target layer must select at least one capability.
        assignments: Vec<LayeredAssignment>,
        #[serde(default)]
        assessments: BTreeMap<String, MilestoneAssessment>,
        reason: String,
        #[serde(default)]
        evidence_execution_ids: Vec<String>,
    },
    Complete {
        #[serde(default)]
        assessments: BTreeMap<String, MilestoneAssessment>,
        reason: String,
        #[serde(default)]
        evidence_execution_ids: Vec<String>,
        /// Final summary bound by a parent plan and shown in the workbench.
        #[serde(default)]
        summary: String,
    },
    Fail {
        reason: String,
        error_type: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredContext {
    #[serde(default)]
    pub run: Option<super::LayeredRun>,
    pub schema_version: u32,
    pub run_id: String,
    pub generation: u64,
    /// The layer to decide (never stored on the run before dispatch).
    pub layer: u32,
    pub total_layers: u32,
    pub request: crate::brain::layered::LayeredRequest,
    pub capabilities: Vec<crate::brain::BrainCapabilityDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todo: Option<crate::brain::layered::LayeredTodoSummary>,
    /// Bounded necessary summaries fetched from the execution owners.
    #[serde(default)]
    pub summaries: BTreeMap<String, String>,
    #[serde(default)]
    pub operations: Vec<crate::brain::layered::LayeredOperation>,
}

/// Root execution's durable dispatch intent: bindings and registered
/// definitions only, never resolved input or child result bodies.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredDispatchIntent {
    pub generation: u64,
    pub layer: u32,
    pub operations: Vec<crate::brain::layered::LayeredOperation>,
    pub assignments: Vec<LayeredAssignment>,
    pub capabilities: Vec<crate::brain::BrainCapabilityDescriptor>,
}

/// Frame a nested run sends its parent when it reaches a terminal phase.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredParentTerminal {
    pub run_id: String,
    pub operation_id: String,
    pub node_id: String,
    pub layer: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<crate::brain::layered::LayeredParent>,
    pub status: crate::brain::layered::LayeredOperationStatus,
    pub source_sequence: u64,
}

impl LayeredOperationStatus {
    /// Child run status folded into the parent operation status.
    pub fn from_phase(phase: LayeredPhase) -> Self {
        match phase {
            LayeredPhase::Completed => Self::Done,
            LayeredPhase::Cancelled => Self::Cancelled,
            LayeredPhase::Failed => Self::Error,
            _ => Self::Running,
        }
    }
}

/// Business assessment is distinct from a capability process exit status.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MilestoneAssessment {
    pub met: bool,
    pub reason: String,
}
