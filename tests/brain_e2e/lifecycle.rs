//! Process-level migration contract for the brain control surface. Historical
//! v2 writes are rejected; v4 is the only public scheduler admission path.

use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::LlmStub;
use serde_json::json;

#[test]
fn raw_brain_submissions_are_rejected_in_favor_of_layered_runs() {
    let stub = LlmStub::spawn(vec![]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "brain-guard-node");
    fleet.wait_ready(&["brain"]);

    let (status, body) = fleet.http(
        "POST",
        "/api/executions",
        &json!({"id":"brain-e2e-bypass","kind":"brain","input":{}}),
    );
    assert_eq!(status, 409, "bypass guard: {body}");
    assert!(body["error"]
        .as_str()
        .unwrap_or_default()
        .contains("schema_version: 4"));
    assert_eq!(stub.request_count(), 0, "no model traffic");
}

#[test]
fn old_run_writes_require_explicit_layered_schema() {
    let stub = LlmStub::spawn(vec![]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet =
        Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "brain-migration-node");
    fleet.wait_ready(&["brain"]);

    for request in [
        json!({"id":"brain-no-version","objective":"legacy"}),
        json!({"id":"brain-v2","schema_version":2,"objective":"legacy"}),
    ] {
        let (status, body) = fleet.http("POST", "/api/brain/runs", &request);
        assert_eq!(status, 409, "request: {body}");
        assert!(body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("schema_version: 4"));
    }
    assert_eq!(stub.request_count(), 0, "migration rejection must not plan");
}
