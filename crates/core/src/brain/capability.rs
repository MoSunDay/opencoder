use crate::fleet::ExecutionKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    Value { value: Value },
    Root { name: String },
    Execution { execution_id: String, path: String },
    Artifact { reference: String },
}
