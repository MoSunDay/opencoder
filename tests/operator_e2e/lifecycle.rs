//! O4 — lifecycle edges of a live operator session: an interrupt during
//! an in-flight drain folds the execution to `interrupted`, and an agent restart between turns does not
//! lose the session — a post-restart prompt drains normally and the
//! transcript spans the restart.

use crate::support::fleet_proc::Fleet;
use crate::support::http_util::sse_read;
use crate::support::llm_stub::{LlmStub, Script, EXTRA_REPLY};
use serde_json::json;

const SESSION: &str = "operator-e2e-lifecycle-1";

#[test]
fn interrupt_mid_drain_and_agent_restart_recovery() {
    // One held request (the interrupt probe) then overflow replies for the
    // recovery turn.
    let stub = LlmStub::spawn(vec![Script::Hold]);
    let tmp = tempfile::tempdir().unwrap();
    let mut fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "op-life-node");
    let (status, body) = fleet.http(
        "POST",
        "/api/sessions",
        &json!({"id": SESSION, "node_id": fleet.node_id(), "agent": "act", "prompt": "long turn"}),
    );
    assert_eq!(status, 200, "create: {body}");

    // The drain is in flight (the LLM request sits inside the stub).
    stub.wait_until_entered();

    // Interrupt through the relay while the model call is parked.
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/sessions/{SESSION}/interrupt"),
        &json!({}),
    );
    assert_eq!(status, 200, "interrupt: {body}");
    let doc = fleet.wait_status(SESSION, "operator interrupted", 30, |doc| {
        doc["execution"]["status"] == "interrupted"
    });
    assert_eq!(
        doc["execution"]["status"], "interrupted",
        "interrupted fold: {doc}"
    );
    assert_eq!(doc["result"], json!({"session_id": SESSION}));
    // The interrupted status surfaced on the event stream too.
    let frames = sse_read(
        &fleet.base,
        &format!("/api/executions/{SESSION}/events"),
        crate::support::fleet_proc::TOKEN,
        0,
        None,
    );
    let interrupted = frames
        .iter()
        .any(|f| f.event == "status" && f.data["status"] == "interrupted");
    assert!(
        interrupted,
        "status frames: {:?}",
        frames.iter().map(|f| &f.event).collect::<Vec<_>>()
    );
    stub.release();

    // Crash + restart the agent; the persisted session survives.
    fleet.respawn_agent();

    // A fresh operator session works after the restart and the OLD
    // session's transcript is still there.
    let (status, detail) = fleet.http("GET", &format!("/api/sessions/{SESSION}"), &json!({}));
    assert_eq!(status, 200, "session survived restart: {detail}");
    assert!(detail["messages"].to_string().contains("long turn"));

    let recovered = "operator-e2e-lifecycle-2";
    let (status, body) = fleet.http(
        "POST",
        "/api/sessions",
        &json!({"id": recovered, "node_id": fleet.node_id(), "agent": "act", "prompt": "after restart"}),
    );
    assert_eq!(status, 200, "post-restart create: {body}");
    let doc = fleet.wait_idle(recovered);
    assert_eq!(doc["execution"]["status"], "idle", "recovered: {doc}");
    let (status, detail) = fleet.http("GET", &format!("/api/sessions/{recovered}"), &json!({}));
    assert_eq!(status, 200);
    assert!(
        detail["messages"].to_string().contains(EXTRA_REPLY),
        "overflow reply: {detail}"
    );
}
