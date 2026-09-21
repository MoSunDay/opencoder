//! Shared layered-canvas fixtures: the two-layer plan, the content-keyed
//! decision responder, the live v3 run the cross-version rule reads, and the
//! read/dispatch helpers every scenario drives.

use crate::support::fleet_proc::{Fleet, TOKEN};
use crate::support::http_util::wait_until;
use crate::support::llm_stub::Script;
use crate::support::{sibling_bin, CLI_BIN};
use serde_json::{json, Value};
use std::process::Command;

/// Run id every layered scenario creates; ids must start with `brain-`.
pub const RUN: &str = "brain-layered-e2e";
/// Run id of the v3 run the cross-version rule reads.
/// What each leaf child answers; the canvas never inspects a child body.
pub const CHILD_TEXT: &str = "e2e-layered-node-result";
/// The closing summary the canvas folds into the run.
pub const SUMMARY: &str = "e2e-layered-canvas-complete";

/// A two-layer canvas: `scan` feeds `apply` through one edge, so the run
/// exercises the layer barrier and the closing activation, not only admission.
/// Same shape as the control-plane fixture, up to the retry overrides.
pub fn plan() -> Value {
    json!({
        "schema_version": 4,
        "title": "layered canvas",
        "objective": "prove the layered canvas through the control plane",
        "nodes": [
            {"node_id":"scan","title":"Scan","capability_id":"builtin-agent-act",
             "retry":{"max_attempts":2}},
            {"node_id":"apply","title":"Apply","capability_id":"builtin-agent-act",
             "retry":{"max_attempts":3}}
        ],
        "edges": [{"from":"scan","to":"apply"}],
        "max_rounds": 8
    })
}

/// The inline-plan submission; `depth`/`parent` stay absent at depth 0.
pub fn request(id: &str) -> Value {
    json!({"id":id,"schema_version":4,"plan":plan(),"inputs":{}})
}

/// Admit one layered root and assert the frozen receipt shape.
pub fn create(fleet: &Fleet, id: &str) -> Value {
    let (status, body) = fleet.http("POST", "/api/brain/runs", &request(id));
    assert_eq!(status, 202, "create layered run: {body}");
    assert_eq!(body["schema_version"], json!(4), "receipt: {body}");
    assert_eq!(body["run_id"], json!(id), "receipt: {body}");
    assert!(body["execution"].is_object(), "receipt: {body}");
    body
}

/// One `/layered` read; the route is the only frozen canvas projection.
pub fn view(fleet: &Fleet, id: &str) -> Value {
    let (status, body) = fleet.http("GET", &format!("/api/brain/runs/{id}/layered"), &json!({}));
    assert_eq!(status, 200, "layered view: {body}");
    body
}

/// One `/layered/rounds/:round` read, status included (misses are contract).
pub fn round(fleet: &Fleet, id: &str, layer: u32) -> (u16, Value) {
    fleet.http(
        "GET",
        &format!("/api/brain/runs/{id}/layered/rounds/{layer}"),
        &json!({}),
    )
}

/// Poll the layered view until `done` accepts it; the phases a healthy canvas
/// never reaches panic with the view attached.
pub fn wait_view(
    fleet: &Fleet,
    id: &str,
    label: &str,
    secs: u64,
    done: impl Fn(&Value) -> bool,
) -> Value {
    let view = wait_until(&fleet.log, label, secs, || {
        let (status, body) =
            fleet.http("GET", &format!("/api/brain/runs/{id}/layered"), &json!({}));
        if status != 200 {
            return None;
        }
        let phase = body["run"]["phase"].as_str()?;
        if done(&body) {
            return Some(body);
        }
        if matches!(phase, "failed" | "cancelled" | "blocked") {
            panic!("{label}: layered run folded to {phase}: {body}");
        }
        None
    });
    view
}

/// Poll the layered view until the run reaches one of `phases`.
pub fn wait_phase(fleet: &Fleet, id: &str, label: &str, secs: u64, phases: &[&str]) -> Value {
    wait_view(fleet, id, label, secs, |view| {
        view["run"]["phase"]
            .as_str()
            .is_some_and(|phase| phases.contains(&phase))
    })
}

/// The operation a node row of a run view names (`None` before dispatch).
pub fn operation<'a>(view: &'a Value, node_id: &str) -> Option<&'a Value> {
    view["operations"]
        .as_array()?
        .iter()
        .find(|op| op["node_id"] == json!(node_id))
}

/// One request-aware responder for the whole canvas. A layered activation
/// carries the one-layer context as its last user message, the closing
/// activation carries `"closing": true`, and every leaf turn carries prose.
/// Nodes and layer come from the context, so the answer cannot drift from the
/// layer being decided.
pub fn responder() -> Script {
    Script::dynamic(|body| {
        let last = body["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|message| message["content"].as_str())
            .unwrap_or_default();
        let context: Value = serde_json::from_str(last).unwrap_or(Value::Null);
        if context["closing"] == json!(true) {
            return json!({
                "decision":"complete",
                "reason":"e2e closing decision after every layer",
                "evidence_execution_ids":[],
                "summary":SUMMARY,
            })
            .to_string();
        }
        if let Some(nodes) = context["nodes"].as_array() {
            let assignments: Vec<Value> = nodes
                .iter()
                .map(|node| {
                    json!({"node_id":node["node_id"],"inputs":{},
                        "reason":"e2e layered dispatch"})
                })
                .collect();
            return json!({
                "decision":"dispatch_layer",
                "layer":context["layer"],
                "assignments":assignments,
                "reason":"e2e layered dispatch",
                "evidence_execution_ids":[],
            })
            .to_string();
        }
        CHILD_TEXT.into()
    })
}

/// Several pops of the same content-keyed responder (two activations, two leaf
/// turns, a best-effort title pass and slack for retries).
pub fn script() -> Vec<Script> {
    vec![responder(); 16]
}

/// The node-local activation runs inside a container, so the live canvas is
/// skipped where runc is absent (same rule as the DAG sandbox suite).
pub fn runc_available() -> bool {
    Command::new("runc")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Drive the real CLI face against the fleet and parse its JSON stdout.
pub fn cli_json(fleet: &Fleet, args: &[&str]) -> Value {
    let output = Command::new(sibling_bin(CLI_BIN))
        .args(["--server", &fleet.base, "--token", TOKEN])
        .args(args)
        .output()
        .expect("spawn opencoder-cli");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "cli {args:?} failed: {stderr}");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "cli {args:?} stdout is not JSON ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}
