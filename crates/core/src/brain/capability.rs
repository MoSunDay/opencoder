use crate::fleet::ExecutionKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BrainCapabilityDescriptor {
    pub capability_id: String,
    pub kind: ExecutionKind,
    pub target: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    pub input_desc: String,
    pub output_desc: String,
    #[serde(default)]
    pub required_inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_outputs: Vec<String>,
    pub definition: Value,
    pub version: String,
}

/// Nodes must advertise this before receiving current capability contracts.
pub const BRAIN_CONTRACT_CAPABILITY: &str = "brain_contracts_v1";
pub const BRAIN_EVIDENCE_MAX_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrainInputBinding {
    Value { value: Value },
    Root { name: String },
    Execution { execution_id: String, path: String },
    Artifact { reference: String },
}
