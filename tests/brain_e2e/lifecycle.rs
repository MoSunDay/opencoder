//! B2 — brain control lanes: the raw-execution bypass guard, the
//! foreign-receipt blocked fold with cancel-to-terminal, and the terminal
//! command gate.

use crate::fixtures;
use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::LlmStub;
use serde_json::json;

/// `POST /api/executions` with `kind=brain` must never reach dispatch: runs
/// are only accepted through `/api/brain/runs` behind immutable plan
/// versions.
#[test]
fn raw_brain_submissions_are_rejected_in_favor_of_plan_versions() {
    let stub = LlmStub::spawn(vec![]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "brain-guard-node");
    fleet.wait_ready(&["brain"]);

    let (status, body) = fleet.http(
        "POST",
        "/api/executions",
        &json!({"id": "brain-e2e-bypass", "kind": "brain", "input": {}}),
    );
    assert_eq!(status, 409, "bypass guard: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("registered immutable plan versions"),
        "bypass marker: {body}"
    );
    assert_eq!(stub.request_count(), 0, "no model traffic");
}

/// A route decision carrying a foreign receipt must block the run; cancel
/// then folds it to `cancelled`, and terminal runs refuse further commands.
#[test]
fn foreign_receipt_blocks_and_cancel_folds_to_terminal() {
    let stub = LlmStub::spawn(fixtures::responder_script(true));
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "brain-block-node");
    fleet.wait_ready(&["brain"]);

    let run_id = "brain-e2e-blocked";
    fixtures::pin_plan(&fleet);
    fixtures::start_run(&fleet, run_id);

    // The child session completes before the route call can be refused.
    let requests = stub.wait_for_requests(3);
    assert!(
        requests[0].contains("Review document"),
        "child prompt: {}",
        requests[0]
    );
    let snapshot = fixtures::wait_phase(
        &fleet,
        run_id,
        "brain run blocks on foreign receipt",
        240,
        &["blocked"],
    );
    assert_eq!(snapshot["phase"], json!("blocked"), "snapshot: {snapshot}");
    assert!(
        snapshot["error"]
            .as_str()
            .unwrap_or_default()
            .contains("foreign receipt"),
        "blocked reason: {snapshot}"
    );
    // The instance itself succeeded; only the route decision was refused.
    let instances = snapshot["instances"].as_array().expect("instances");
    assert_eq!(
        instances[0]["status"],
        json!("succeeded"),
        "instances: {snapshot}"
    );

    // Cancel folds the blocked run through cancelling into cancelled.
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/brain/runs/{run_id}/commands"),
        &json!({"action": "cancel"}),
    );
    assert_eq!(status, 200, "cancel: {body}");
    let snapshot = fixtures::wait_phase(&fleet, run_id, "brain run cancels", 120, &["cancelled"]);
    assert_eq!(
        snapshot["phase"],
        json!("cancelled"),
        "snapshot: {snapshot}"
    );
    let doc = fleet.wait_terminal(run_id);
    assert_eq!(
        doc["execution"]["status"],
        json!("cancelled"),
        "inspect: {doc}"
    );

    // Terminal runs refuse further commands.
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/brain/runs/{run_id}/commands"),
        &json!({"action": "cancel"}),
    );
    // The node folds the refused ensure! into a 500 RPC reply.
    assert_eq!(status, 500, "terminal command: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("run is terminal; create a new run to execute again"),
        "terminal marker: {body}"
    );
}
