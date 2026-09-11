//! DAG workflow spec — the persisted shape (`dag_defs.spec_json` and the
//! per-run snapshot `dag_runs.spec_json`).
//!
//! A spec is a named list of steps plus dependency edges (`depends_on`).
//! Two step kinds exist today:
//! - `agent` — the step prompt runs through a full local session runner on
//!   the executing node (same executor the node-task path uses).
//! - `wasm`  — the step runs a WebAssembly module (`command` = the module
//!   path plus args) in the embedded wasm runtime by default, or inside an
//!   `runc` container when `sandbox: "runc"` is set.
//!
//! Protocol note: the `python` step kind was REMOVED (breaking protocol
//! change). Old specs containing python steps fail to decode with a
//! dedicated message — see [`crate::decode_spec`]; there is no silent
//! migration.
//!
//! Specs are plain data with serde defaults so old snapshots keep decoding
//! when new optional fields appear.

use serde::{Deserialize, Serialize};

/// Upper bound for `how_append` payloads (bytes, UTF-8).
pub const MAX_HOW_APPEND_BYTES: usize = 8 * 1024;

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
    /// Optional per-step wall-clock budget in seconds (agent + wasm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

/// What a step executes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepKind {
    /// Administrator-registered binary workflow; no caller-controlled command.
    Runner { runner: String, agent: String },
    /// Run a prompt through the local session runner on the node.
    Agent {
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// Workflow-author-declared knowledge fragment. Injected into the
        /// step session's child-process environment as
        /// `OPENCODER_HOW_APPEND`; after the step finishes successfully the
        /// node appends the declared value to the step agent's shared-pool
        /// `how.md` (a new versioned resource, never a bare file write).
        /// Bounded by [`MAX_HOW_APPEND_BYTES`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        how_append: Option<String>,
    },
    /// Run a WebAssembly module on the node. `command` is the launch
    /// command: `"<module.wasm> [args...]"` (whitespace-split; the module
    /// path is relative to the run's context root). The module token may
    /// pin an explicit pool version — `tool@v3.wasm` freezes version 3 of
    /// the `tool` pool module at accept time, while plain `tool.wasm`
    /// takes the pool `current` (see the `opencode-dag-wasm` token
    /// grammar). Default sandbox is the
    /// embedded wasm runtime; [`SandboxMode::Runc`] wraps the step in an OCI
    /// container (`runc run`) with the run directory bind-mounted at
    /// `/workspace/context`.
    Wasm {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sandbox: Option<SandboxMode>,
    },
}

/// Execution sandbox for wasm steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// Embedded wasm runtime (wasmtime) in the agent process (default).
    #[default]
    InProcess,
    /// `runc` container: rootfs readonly, `/workflow/<run_id>` bind-mounted
    /// read-write at `/workspace/context`.
    Runc,
}

/// Decode a spec from a JSON value, mapping the removed `python` step kind
/// to a dedicated migration error instead of a raw serde variant message.
/// No silent migration happens: the caller surfaces the error to the user.
pub fn decode_spec(value: &serde_json::Value) -> Result<DagSpec, String> {
    if let Some(steps) = value.get("steps").and_then(|s| s.as_array()) {
        for step in steps {
            if step
                .get("kind")
                .and_then(|k| k.get("type"))
                .and_then(|t| t.as_str())
                == Some("python")
            {
                return Err(
                    "该定义使用已下线的 python 步骤，请改写为 wasm/agent 后重新保存".to_string(),
                );
            }
        }
    }
    serde_json::from_value(value.clone()).map_err(|e| e.to_string())
}

/// Decode a spec from a JSON string — parse first, then [`decode_spec`], so
/// every persisted-spec read path (`dag_defs.spec_json`, inline todo
/// executor specs) reports the same dedicated python migration error
/// instead of a raw serde variant message.
pub fn decode_spec_str(raw: &str) -> Result<DagSpec, String> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    decode_spec(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Round-trip a representative spec and prove the wire shape is the
    /// documented one (snake_case tag, optional fields omitted).
    /// The string decoder shares the python migration sentinel and maps
    /// plain JSON syntax errors to strings (callers format them as-is).
    #[test]
    fn decode_spec_str_maps_python_and_syntax_errors() {
        let python =
            r#"{"name":"x","steps":[{"name":"a","kind":{"type":"python","code":"pass"}}]}"#;
        let err = decode_spec_str(python).unwrap_err();
        assert!(err.contains("已下线的 python 步骤"), "{err}");

        // A plain JSON syntax error stays a positional serde message.
        assert!(decode_spec_str("{not json").unwrap_err().contains("column"));

        let ok = decode_spec_str(
            r#"{"name":"x","steps":[{"name":"a","kind":{"type":"wasm","command":"m.wasm"}}]}"#,
        )
        .unwrap();
        assert_eq!(ok.steps.len(), 1);
    }

    #[test]
    fn spec_roundtrip_wire_shape() {
        let v = json!({
            "name": "etl",
            "steps": [
                { "name": "fetch", "kind": { "type": "wasm", "command": "tool.wasm --flag" } },
                { "name": "review", "depends_on": ["fetch"],
                  "kind": { "type": "agent", "prompt": "review", "how_append": "note" } },
                { "name": "containerized", "depends_on": ["fetch"],
                  "kind": { "type": "wasm", "command": "tool.wasm", "sandbox": "runc" } }
            ]
        });
        let spec: DagSpec = serde_json::from_value(v).unwrap();
        assert_eq!(spec.steps.len(), 3);
        assert_eq!(
            spec.steps[0].kind,
            StepKind::Wasm {
                command: "tool.wasm --flag".into(),
                sandbox: None
            }
        );
        assert_eq!(
            spec.steps[1].kind,
            StepKind::Agent {
                prompt: "review".into(),
                agent: None,
                model: None,
                how_append: Some("note".into())
            }
        );
        assert_eq!(
            spec.steps[2].kind,
            StepKind::Wasm {
                command: "tool.wasm".into(),
                sandbox: Some(SandboxMode::Runc)
            }
        );
        // Round-trip.
        let back: DagSpec = serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(back, spec);
        // Defaults: sandbox omitted decodes as None (embedded runtime).
        assert!(serde_json::to_string(&spec.steps[0])
            .unwrap()
            .contains("\"command\":\"tool.wasm --flag\""));
        // how_append is omitted when None (old agent specs keep decoding).
        assert!(!serde_json::to_string(&spec.steps[0])
            .unwrap()
            .contains("how_append"));
        let legacy: DagSpec = serde_json::from_value(json!({
            "name": "old", "steps": [ { "name": "a",
              "kind": { "type": "agent", "prompt": "p", "agent": "act" } } ]
        }))
        .unwrap();
        assert!(matches!(
            legacy.steps[0].kind,
            StepKind::Agent {
                how_append: None,
                ..
            }
        ));
    }

    #[test]
    fn sandbox_default_is_in_process() {
        assert_eq!(SandboxMode::default(), SandboxMode::InProcess);
    }

    #[test]
    fn decode_spec_maps_removed_python_to_migration_error() {
        let err = decode_spec(&json!({
            "name": "old", "steps": [
                { "name": "a", "kind": { "type": "python", "code": "print(1)" } } ]
        }))
        .unwrap_err();
        assert!(err.contains("python"), "{err}");
        assert!(err.contains("wasm"), "{err}");
        // wasm/agent specs decode through the helper unchanged.
        assert!(decode_spec(&json!({
            "name": "ok", "steps": [ { "name": "a", "kind": { "type": "wasm", "command": "m.wasm" } } ]
        }))
        .is_ok());
        // A python step nested anywhere in the list is caught (pre-scan),
        // even when serde would fail on something else first.
        let err = decode_spec(&json!({
            "name": "mixed", "steps": [
                { "name": "a", "kind": { "type": "agent", "prompt": "p" } },
                { "name": "b", "kind": { "type": "python", "code": "x", "sandbox": "runc" } } ]
        }))
        .unwrap_err();
        assert!(err.contains("python"), "{err}");
    }
}
