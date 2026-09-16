//! Shared brain fixtures: the immutable plan version, the run request, the
//! content-keyed stub responder, and the pin/start/wait helpers shared by
//! both scenarios.

use crate::support::fleet_proc::Fleet;
use crate::support::http_util::wait_until;
use crate::support::llm_stub::Script;
use serde_json::{json, Value};

pub const PLAN_ID: &str = "plan-e2e";
pub const REPORT_MARKER: &str = "e2e-node-owned-report";
/// The forged receipt the blocked scenario answers with.
pub const FORGED_RECEIPT: &str = "receipt-forged-by-stub";

/// Immutable plan: one agent instance producing `report`, one terminating
/// route whose single exit requires a completed and verified delivery.
/// Same contract as `crates/worker/tests/support/brain.rs::plan`.
pub fn plan() -> Value {
    json!({
        "schema_version": 2,
        "title": "Review",
        "objective": "verified report",
        "inputs": {"document": {
            "description": "Named document",
            "source": {"kind": "external"},
            "schema": {"type": "object"},
            "required": true,
        }},
        "instances": [{
            "id": "one",
            "description": "Review evidence",
            "capability_id": "builtin-agent-act",
            "action": {"kind": "agent", "target": "act", "prompt": "Review document", "output_mode": "json"},
            "inputs": ["document"],
            "outputs": ["report"],
        }],
        "outputs": {"report": {"description": "Verified report"}},
        "entry": ["one"],
        "routes": [{
            "id": "finish",
            "description": "Deliver verified output",
            "outputs": ["report"],
            "targets": [],
            "exits": [{
                "id": "done",
                "description": "Deliver report",
                "deliverables": ["report"],
                "require_completed": true,
                "require_verified": true,
            }],
        }],
    })
}

/// The `POST /api/brain/plan-defs` body (a `PlanVersion`).
pub fn plan_version() -> Value {
    json!({
        "id": PLAN_ID,
        "version": 1,
        "created_at": 1,
        "changelog": "e2e fixture",
        "plan": plan(),
    })
}

pub fn run_inputs() -> Value {
    json!({"document": {"name": "Requirement", "markdown": "# Review"}})
}

/// The fixed-mode run request; a replay must send the identical intent.
pub fn run_request(id: &str) -> Value {
    json!({
        "id": id,
        "mode": "fixed",
        "objective": "verified report",
        "inputs": run_inputs(),
        "plan": {"id": PLAN_ID, "version": 1},
    })
}

/// The child agent session's reply: the named-output envelope the brain
/// runtime parses back into produced-output records.
pub fn agent_envelope() -> String {
    json!({
        "report": {
            "content": REPORT_MARKER,
            "completion": {"passed": true, "evidence": ["delivered"]},
            "verification": {"passed": true, "evidence": ["e2e verified"]},
        }
    })
    .to_string()
}

/// One request-aware responder for the whole run, keyed on the wire
/// contracts rather than request order — the child session's best-effort
/// title pass replays the same user message and must not eat the route
/// slot. Requests carrying `Named output descriptions` (the child turn and
/// the title pass) get the output envelope; the route call carries the
/// RouteContext JSON as its last message and gets a receipt-echoing (or,
/// with `foreign`, forged) decision.
pub fn run_responder(foreign: bool) -> Script {
    Script::dynamic(move |body| {
        let last = body["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|message| message["content"].as_str())
            .unwrap_or_default();
        let context: Value = serde_json::from_str(last).unwrap_or(Value::Null);
        if context.get("receipt").is_some() {
            let receipt = if foreign {
                json!(FORGED_RECEIPT)
            } else {
                context["receipt"].clone()
            };
            return json!({
                "receipt": receipt,
                "reason": "e2e route decision",
                "selected": [],
                "exit": context.pointer("/exits/0/id").cloned().unwrap_or(Value::Null),
            })
            .to_string();
        }
        agent_envelope()
    })
}

/// The responder script: several pops of the same content-keyed closure
/// (the child turn, its title pass, the route decision, and slack for
/// retries); requests beyond it fall to the stub's EXTRA_REPLY.
pub fn responder_script(foreign: bool) -> Vec<Script> {
    vec![run_responder(foreign); 8]
}

/// Pin the plan version (the only supported submission path for brain runs).
pub fn pin_plan(fleet: &Fleet) {
    let (status, body) = fleet.http("POST", "/api/brain/plan-defs", &plan_version());
    assert_eq!(status, 200, "pin plan version: {body}");
    assert_eq!(body["version"]["id"], json!(PLAN_ID), "pin: {body}");
    assert_eq!(body["version"]["version"], json!(1), "pin: {body}");
}

/// Start a fixed-mode run and assert the accepted dispatch envelope.
pub fn start_run(fleet: &Fleet, id: &str) {
    let (status, body) = fleet.http("POST", "/api/brain/runs", &run_request(id));
    assert_eq!(status, 202, "start brain run: {body}");
    assert_eq!(body["id"], json!(id), "dispatch: {body}");
    assert_eq!(body["kind"], json!("brain"), "dispatch: {body}");
}

/// Poll the run snapshot until `done` lists the phase; folding to an
/// unexpected terminal/blocked phase panics with the snapshot attached.
pub fn wait_phase(fleet: &Fleet, id: &str, label: &str, secs: u64, done: &[&str]) -> Value {
    wait_until(&fleet.log, label, secs, || {
        let (status, body) = fleet.http("GET", &format!("/api/brain/runs/{id}"), &json!({}));
        if status != 200 {
            return None;
        }
        let phase = body["phase"].as_str()?;
        if done.contains(&phase) {
            return Some(body);
        }
        if matches!(phase, "blocked" | "failed" | "cancelled" | "completed") {
            panic!("{label}: run folded to {phase}: {body}");
        }
        None
    })
}
