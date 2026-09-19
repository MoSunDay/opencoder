//! B1 — the full brain face: pin an immutable plan version, start a
//! fixed-mode run, and watch the real node complete it — the child agent
//! session runs against the LLM stub, the receipt-bound route picks the
//! exit, and the accepted intent replays idempotently.

use crate::fixtures;
use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::LlmStub;
use serde_json::{json, Value};

const RUN_ID: &str = "brain-e2e-fixed";

#[test]
fn fixed_plan_run_completes_through_child_session_and_receipt_route() {
    // One content-keyed responder serves the child session (and its title
    // pass) plus the receipt-bound route decision.
    let stub = LlmStub::spawn(fixtures::responder_script(false));
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "brain-flow-node");
    fleet.wait_ready(&["brain"]);

    fixtures::pin_plan(&fleet);
    fixtures::start_run(&fleet, RUN_ID);

    // Three model calls: the child session, its best-effort title pass, and
    // the route decision. The route call is the one with the routing system
    // prompt; the child prompt carries the plan action and output contract.
    let requests = stub.wait_for_requests(3);
    assert!(
        requests[0].contains("Review document") && requests[0].contains("You are OpenCoder"),
        "child prompt must open the session with the plan action: {}",
        requests[0]
    );
    assert!(
        requests[0].contains("Named output descriptions"),
        "child prompt must carry the output contract: {}",
        requests[0]
    );
    let route = requests
        .iter()
        .find(|request| request.contains("local output router"))
        .expect("route decision call");
    let route_body: Value = serde_json::from_str(route).expect("route request json");
    let context: Value =
        serde_json::from_str(route_body["messages"][1]["content"].as_str().unwrap())
            .expect("RouteContext json");
    assert_eq!(
        context["route"],
        json!("finish"),
        "route context: {context}"
    );
    assert!(
        context["receipt"]
            .as_str()
            .unwrap_or_default()
            .starts_with("route-"),
        "route context carries the prepared receipt: {context}"
    );

    let snapshot = fixtures::wait_phase(&fleet, RUN_ID, "brain run completes", 240, &["completed"]);
    assert_eq!(
        snapshot["phase"],
        json!("completed"),
        "snapshot: {snapshot}"
    );
    assert_eq!(snapshot["mode"], json!("fixed"), "snapshot: {snapshot}");
    assert!(
        snapshot["watermark"].as_u64().unwrap_or(0) > 0,
        "events watermark: {snapshot}"
    );
    // The run folded cleanly: exactly the child turn, the title pass, and
    // the route decision hit the model.
    assert_eq!(stub.request_count(), 3, "requests: {requests:?}");

    let instances = snapshot["instances"].as_array().expect("instances");
    assert_eq!(instances.len(), 1, "instances: {snapshot}");
    assert_eq!(
        instances[0]["step_id"],
        json!("one"),
        "instances: {snapshot}"
    );
    assert_eq!(
        instances[0]["status"],
        json!("succeeded"),
        "instances: {snapshot}"
    );
    let child = instances[0]["execution"]["id"]
        .as_str()
        .expect("child execution id")
        .to_string();
    assert!(
        child.starts_with("agent-") && child.len() == "agent-".len() + 40,
        "child execution id is a fingerprint: {child}"
    );

    // The child execution is a real agent session: the inspect document
    // exposes it, and its decoded transcript carries the brain output
    // contract plus the stub's envelope reply.
    let (_, doc) = fleet.http("GET", &format!("/api/executions/{child}"), &json!({}));
    assert_eq!(doc["session"]["meta"]["id"], json!(child), "inspect: {doc}");
    let (status, detail) = fleet.http("GET", &format!("/api/sessions/{child}"), &json!({}));
    assert_eq!(status, 200, "child session detail: {detail}");
    assert_eq!(detail["id"], json!(child), "session detail: {detail}");
    let transcript = detail["messages"].to_string();
    assert!(
        transcript.contains("Named output descriptions"),
        "child transcript must carry the output contract: {transcript}"
    );
    assert!(
        transcript.contains(fixtures::REPORT_MARKER),
        "child transcript must carry the stub reply: {transcript}"
    );

    // Replay: the accepted intent is idempotent — 202 with zero extra calls.
    let before = stub.request_count();
    let (status, body) = fleet.http("POST", "/api/brain/runs", &fixtures::run_request(RUN_ID));
    assert_eq!(status, 202, "replay: {body}");
    assert_eq!(
        stub.request_count(),
        before,
        "replay must not reach the model again"
    );

    // The event page is populated and closed.
    let (status, page) = fleet.http(
        "GET",
        &format!("/api/brain/runs/{RUN_ID}/events-page?after=0"),
        &json!({}),
    );
    assert_eq!(status, 200, "events page: {page}");
    assert!(
        !page["events"].as_array().expect("events").is_empty(),
        "events: {page}"
    );
    assert_eq!(
        page["more"],
        json!(false),
        "events page fully drained: {page}"
    );
}
