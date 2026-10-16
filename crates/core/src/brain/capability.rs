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
pub const EXECUTION_IO_CAPABILITY: &str = "execution_io_v1";
pub const BRAIN_EVIDENCE_MAX_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrainExecutionSource {
    #[default]
    Output,
    Artifacts,
}

impl BrainExecutionSource {
    fn is_output(&self) -> bool {
        *self == Self::Output
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrainInputBinding {
    Value {
        value: Value,
    },
    Root {
        name: String,
    },
    Execution {
        execution_id: String,
        path: String,
        #[serde(default, skip_serializing_if = "BrainExecutionSource::is_output")]
        source: BrainExecutionSource,
    },
    Artifact {
        reference: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn execution_sources_preserve_old_bindings_and_reject_unknown_sources() {
        let old = json!({"kind":"execution","execution_id":"agent-one","path":"/passed"});
        let binding: BrainInputBinding = serde_json::from_value(old.clone()).unwrap();
        assert!(matches!(
            binding,
            BrainInputBinding::Execution {
                source: BrainExecutionSource::Output,
                ..
            }
        ));
        assert_eq!(serde_json::to_value(binding).unwrap(), old);
        let files =
            json!({"kind":"execution","execution_id":"dag-one","source":"artifacts","path":"/0"});
        let binding: BrainInputBinding = serde_json::from_value(files.clone()).unwrap();
        assert!(matches!(
            binding,
            BrainInputBinding::Execution {
                source: BrainExecutionSource::Artifacts,
                ..
            }
        ));
        assert_eq!(serde_json::to_value(binding).unwrap(), files);
        assert!(serde_json::from_value::<BrainInputBinding>(
            json!({"kind":"execution","execution_id":"dag-one","source":"files","path":""})
        )
        .is_err());
    }
}
