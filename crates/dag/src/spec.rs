//! DAG workflow spec — the persisted shape (`dag_defs.spec_json` and the
//! per-run snapshot `dag_runs.spec_json`).
//!
//! A spec is a named list of steps plus dependency edges (`depends_on`).
//! Two step kinds exist today:
//! - `agent`  — the step prompt runs through a full local session runner on
//!   the executing node (same executor the node-task path uses).
//! - `python` — the step code runs on the node in the embedded RustPython VM
//!   by default, or inside an `runc` container when `sandbox: "runc"` is set.
//!
//! Specs are plain data with serde defaults so old snapshots keep decoding
//! when new optional fields appear.

use serde::{Deserialize, Serialize};

/// Whole-workflow definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DagSpec {
    /// Human-readable workflow name (not required to be a slug; the def id is).
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub steps: Vec<StepSpec>,
}

/// One node in the workflow graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepSpec {
    /// Step slug: `[a-z0-9][a-z0-9-]{0,63}` — also its artifacts directory
    /// name under `/workflow/<run_id>/`, so it must be path-safe.
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    pub kind: StepKind,
    /// Optional per-step wall-clock budget in seconds (agent + python).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

/// What a step executes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepKind {
    /// Run a prompt through the local session runner on the node.
    Agent {
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    /// Run python code on the node. Default sandbox is the embedded
    /// RustPython VM; [`SandboxMode::Runc`] wraps the step in an OCI
    /// container (`runc run`) with the run directory bind-mounted at
    /// `/workspace/context`.
    Python {
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sandbox: Option<SandboxMode>,
    },
}

/// Execution sandbox for python steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// Embedded RustPython VM in the agent process (default).
    #[default]
    InProcess,
    /// `runc` container: rootfs readonly, `/workflow/<run_id>` bind-mounted
    /// read-write at `/workspace/context`.
    Runc,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Round-trip a representative spec and prove the wire shape is the
    /// documented one (snake_case tag, optional fields omitted).
    #[test]
    fn spec_roundtrip_wire_shape() {
        let v = json!({
            "name": "etl",
            "steps": [
                { "name": "fetch", "kind": { "type": "python", "code": "print(1)" } },
                { "name": "review", "depends_on": ["fetch"],
                  "kind": { "type": "agent", "prompt": "review" } },
                { "name": "containerized", "depends_on": ["fetch"],
                  "kind": { "type": "python", "code": "print(2)", "sandbox": "runc" } }
            ]
        });
        let spec: DagSpec = serde_json::from_value(v).unwrap();
        assert_eq!(spec.steps.len(), 3);
        assert_eq!(
            spec.steps[0].kind,
            StepKind::Python {
                code: "print(1)".into(),
                sandbox: None
            }
        );
        assert_eq!(
            spec.steps[1].kind,
            StepKind::Agent {
                prompt: "review".into(),
                agent: None,
                model: None
            }
        );
        assert_eq!(
            spec.steps[2].kind,
            StepKind::Python {
                code: "print(2)".into(),
                sandbox: Some(SandboxMode::Runc)
            }
        );
        // Round-trip.
        let back: DagSpec = serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(back, spec);
        // Defaults: sandbox omitted decodes as None (VM).
        assert!(serde_json::to_string(&spec.steps[0])
            .unwrap()
            .contains("\"code\":\"print(1)\""));
    }

    #[test]
    fn sandbox_default_is_in_process() {
        assert_eq!(SandboxMode::default(), SandboxMode::InProcess);
    }
}
