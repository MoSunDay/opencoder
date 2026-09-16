//! O2 — the `/api/sessions/:id/*` relay fallback: every session operation
//! rides the execution command channel as action `http`
//! (`{method, tail, body}`). Covered: a follow-up prompt drives a second
//! drain, GETs read live state, and the guard rails hold (traversal,
//! empty id and non-allow-listed methods are refused with the documented
//! 400s; unknown sessions are 404s).

use crate::support::fleet_proc::{Fleet, TOKEN};
use crate::support::http_util::sse_read;
use crate::support::llm_stub::{LlmStub, EXTRA_REPLY};
use serde_json::json;

const REPLY: &str = "e2e-relay-reply";
const SESSION: &str = "operator-e2e-relay-1";

#[test]
fn relay_followup_prompt_reads_and_rejections() {
    // Two scripted replies: the create-time prompt and the relay prompt.
    let stub = LlmStub::spawn_text(&[REPLY, EXTRA_REPLY]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "op-relay-node");
    let (status, body) = fleet.http(
        "POST",
        "/api/sessions",
        &json!({"id": SESSION, "node_id": fleet.node_id(), "agent": "act", "prompt": "warm up"}),
    );
    assert_eq!(status, 200, "create: {body}");
    fleet.wait_idle(SESSION);

    // Follow-up prompt through the relay (the native `/prompt` endpoint,
    // driven by command action `http`).
    let follow_up = "relay-followup-prompt";
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/sessions/{SESSION}/prompt"),
        &json!({"prompt": follow_up, "input_id": "e2e-followup"}),
    );
    assert_eq!(status, 200, "relay prompt: {body}");
    assert_eq!(body["driver_ensured"], json!(true), "drain started: {body}");
    fleet.wait_idle(SESSION);

    // The second reply landed: the transcript now has both turns and the
    // stub saw both prompts.
    let (status, detail) = fleet.http("GET", &format!("/api/sessions/{SESSION}"), &json!({}));
    assert_eq!(status, 200);
    let transcript = detail["messages"].to_string();
    assert!(
        transcript.contains(REPLY) && transcript.contains(EXTRA_REPLY),
        "{transcript}"
    );
    let requests = stub.wait_for_requests(2);
    assert!(
        requests[1].contains(follow_up),
        "follow-up: {}",
        requests[1]
    );

    // Guard rails: path traversal and an empty session id are refused by
    // the control relay before any node round-trip.
    for path in [
        format!("/api/sessions/{SESSION}/../../etc/passwd"),
        format!("/api/sessions//{SESSION}"),
    ] {
        let (status, body) = fleet.http("GET", &path, &json!({}));
        assert_eq!(status, 400, "guard {path}: {body}");
        assert!(
            body["error"]
                .as_str()
                .unwrap_or_default()
                .contains("session path"),
            "guard text: {body}"
        );
    }
    // Method outside the node's allow-list (GET|POST|PATCH|DELETE): the
    // node rejects the operation itself.
    let (status, body) = fleet.http("PUT", &format!("/api/sessions/{SESSION}/model"), &json!({}));
    assert_eq!(status, 400, "PUT via relay: {body}");
    assert_eq!(body["error"], "invalid session operation");
    // Unknown session id: the execution index answers 404.
    let (status, body) = fleet.http("GET", "/api/sessions/unknown-session-xyz", &json!({}));
    assert_eq!(status, 404, "unknown session: {body}");
    assert_eq!(body["error"], "execution id not found");

    // The replayed stream carries both turns and ends terminally.
    let frames = sse_read(
        &fleet.base,
        &format!("/api/sessions/{SESSION}/events"),
        TOKEN,
        0,
        None,
    );
    assert_eq!(frames.last().unwrap().event, "stream_end");
    let all = frames
        .iter()
        .map(|f| f.data.to_string())
        .collect::<String>();
    assert!(
        all.contains(REPLY) && all.contains(EXTRA_REPLY),
        "sse: {all}"
    );
}
