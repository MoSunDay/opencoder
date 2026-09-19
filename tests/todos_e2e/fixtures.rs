//! Shared TODO fixtures: the template spec, the install helper, the scripted
//! parent/child replies (same JSON contracts as
//! `crates/worker/tests/workloads.rs`) and small assertion helpers.

use crate::support::fleet_proc::Fleet;
use serde_json::{json, Value};

pub const TEMPLATE: &str = "demo";
pub const TODO_ID: &str = "t1";

/// Parent decision #1: dispatch the single todo in a fresh session.
pub const PARENT_DISPATCH: &str =
    r#"{"operation":"dispatch","todos":[{"todo_id":"t1","context_mode":"new"}],"reason":"ready"}"#;

/// Child reply: a candidate acceptance payload.
pub const CHILD_CANDIDATE: &str = r#"{"status":"candidate","summary":"done","result":"ok","verification":"checked","evidence_refs":[],"recovery_context":{"summary":"done","refs":[]}}"#;

/// Parent decision #2: accept the candidate and mark the milestone.
pub const PARENT_ACCEPT: &str =
    r#"{"operation":"accept","reason":"meets criteria","mark_milestone":true}"#;

/// Parent decision #3: complete the workflow.
pub const PARENT_COMPLETE: &str = r#"{"operation":"complete","reason":"all passed"}"#;

/// Template spec — same shape as the control-plane e2e fixture. The single
/// todo pins `max_attempts: 1` so the failure scenario folds deterministically.
pub fn spec() -> Value {
    json!({
        "schema_version": 1,
        "id": "wf-demo",
        "name": TEMPLATE,
        "objective": "ship it",
        "todos": [{
            "id": TODO_ID, "title": "T1", "requirement_background": "bg",
            "instructions": "do it", "agent": "act", "max_attempts": 1,
            "acceptance": {"criteria": "acceptance-criteria-mark"},
        }],
        "metadata": {},
    })
}

/// The exactly-four FIFO replies driving a happy workflow.
pub fn stub_script() -> Vec<crate::support::llm_stub::Script> {
    use crate::support::llm_stub::Script;
    vec![
        Script::Text(PARENT_DISPATCH.into()),
        Script::Text(CHILD_CANDIDATE.into()),
        Script::Text(PARENT_ACCEPT.into()),
        Script::Text(PARENT_COMPLETE.into()),
    ]
}

/// Save the template (asserting the documented v1 pin) and refuse duplicates.
pub fn install_template(fleet: &Fleet, name: &str) {
    let (status, body) = fleet.http(
        "POST",
        "/api/todo/templates",
        &json!({"name": name, "spec": spec()}),
    );
    assert_eq!(status, 200, "create template: {body}");
    assert_eq!(
        body["template"]["current"],
        json!("v1"),
        "template pin: {body}"
    );
    let revision: Value =
        serde_json::from_str(body["revision"].as_str().expect("revision json string"))
            .expect("revision payload");
    assert_eq!(revision["current"], json!("v1"), "revision: {revision}");
    let (status, body) = fleet.http(
        "POST",
        "/api/todo/templates",
        &json!({"name": name, "spec": spec()}),
    );
    assert_eq!(status, 409, "duplicate template: {body}");
}

/// `sub` appears in `kinds` in order.
pub fn assert_subsequence(kinds: &[&str], sub: &[&str], context: &str) {
    let mut cursor = 0;
    for needle in sub {
        let found = kinds[cursor..]
            .iter()
            .position(|kind| kind == needle)
            .unwrap_or_else(|| {
                panic!("{context}: expected {needle:?} after {cursor} in {kinds:?}")
            });
        cursor += found + 1;
    }
}

/// Read a node artifact and parse it as JSON.
pub fn read_json(path: &std::path::Path) -> Value {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}
