//! O8 — inputs admitted while an agent round is actively running: a steer
//! and a queue prompt posted mid-turn stay in the session durably — the
//! same `input_id` + payload replays to the same `admitted_seq` with no
//! duplicate row, the same `input_id` with a different payload is refused
//! (409), and after the in-flight turn ends BOTH pending inputs reach the
//! model exactly once: the steer is promoted into the next round and the
//! queued task drains as its own follow-up round.

use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::{LlmStub, Script};
use serde_json::json;
use std::time::{Duration, Instant};

const SESSION: &str = "agent-e2e-active-inputs-1";

/// User texts of every conversation round the stub served, in order. A
/// round is a model request without a `max_tokens` cap: the async title
/// side call (`harness/title.rs`, capped at 64 tokens) shares the stub but
/// is not a round, so it is filtered instead of racing its arrival.
fn rounds(stub: &LlmStub) -> Vec<Vec<String>> {
    stub.wait_for_requests(stub.request_count())
        .iter()
        .filter_map(|body| serde_json::from_str::<serde_json::Value>(body).ok())
        .filter(|request| request["max_tokens"].is_null())
        .map(|request| {
            request["messages"]
                .as_array()
                .map(|messages| {
                    messages
                        .iter()
                        .filter(|message| message["role"] == "user")
                        .filter_map(|message| message["content"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect()
}

/// Poll until at least `n` rounds are recorded (a round can trail the
/// execution fold, e.g. the follow-up drain after admission).
fn wait_for_rounds(stub: &LlmStub, n: usize) -> Vec<Vec<String>> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let recorded = rounds(stub);
        if recorded.len() >= n {
            return recorded;
        }
        assert!(
            Instant::now() < deadline,
            "only {n} rounds after 30s: {recorded:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn active_round_inputs_steer_queue_idempotent_and_promoted() {
    // The first round parks inside the stub; the promoted rounds read the
    // scripted reply and the stub's overflow default, so no extra call can
    // wedge the drain.
    let stub = LlmStub::spawn(vec![Script::Hold, Script::Text("round two reply".into())]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "op-active-inputs");
    fleet.wait_ready(&["operator", "dag", "agent"]);

    let (status, body) = fleet.http(
        "POST",
        "/api/sessions",
        &json!({"id": SESSION, "kind": "agent", "agent": "act",
                "node_id": fleet.node_id(), "prompt": "first task"}),
    );
    assert_eq!(status, 200, "create: {body}");
    assert!(SESSION.starts_with("agent-"), "agent lane id prefix");
    stub.wait_until_entered(); // the round is in flight (parked in the stub)

    // Steer mid-turn: idempotent retry, conflicting payload refused.
    let steer = json!({"prompt": "extra context", "input_id": "guidance", "delivery": "steer"});
    let (status, first) = fleet.http("POST", &format!("/api/sessions/{SESSION}/prompt"), &steer);
    assert_eq!(status, 200, "steer admit: {first}");
    assert_eq!(first["ok"], json!(true), "{first}");
    let steer_seq = first["admitted_seq"].as_i64().unwrap();
    let (status, retry) = fleet.http("POST", &format!("/api/sessions/{SESSION}/prompt"), &steer);
    assert_eq!(status, 200, "steer retry: {retry}");
    assert_eq!(retry["admitted_seq"], json!(steer_seq), "same row: {retry}");
    let (status, conflict) = fleet.http(
        "POST",
        &format!("/api/sessions/{SESSION}/prompt"),
        &json!({"prompt": "different payload", "input_id": "guidance", "delivery": "steer"}),
    );
    assert_eq!(status, 409, "input id reuse with a new payload: {conflict}");
    assert!(
        conflict["error"]
            .as_str()
            .is_some_and(|e| e.contains("already exists for session")),
        "{conflict}"
    );

    // A queued task lands while the same round is still parked.
    let queued = json!({"prompt": "second task", "input_id": "queued", "delivery": "queue"});
    let (status, qbody) = fleet.http("POST", &format!("/api/sessions/{SESSION}/prompt"), &queued);
    assert_eq!(status, 200, "queue admit: {qbody}");
    let queued_seq = qbody["admitted_seq"].as_i64().unwrap();

    stub.release();
    let doc = fleet.wait_idle(SESSION);
    assert_eq!(doc["execution"]["node_id"], fleet.node_id(), "node fold: {doc}");

    // The transcript holds each prompt exactly once (no loss, no duplicates).
    let (status, detail) = fleet.http("GET", &format!("/api/sessions/{SESSION}"), &json!({}));
    assert_eq!(status, 200, "session read: {detail}");
    let user_texts: Vec<&str> = detail["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "user")
        .flat_map(|message| message["blocks"].as_array().unwrap())
        .filter_map(|block| block["text"].as_str())
        .collect();
    assert_eq!(user_texts.len(), 3, "user turns: {detail}");
    for prompt in ["first task", "extra context", "second task"] {
        assert_eq!(
            user_texts.iter().filter(|text| **text == prompt).count(),
            1,
            "{detail}"
        );
    }

    // Both pending inputs reached the model exactly once, in order: the
    // steer is promoted into the round after the in-flight one, the queued
    // task drains as its own follow-up round with the full history.
    let promoted = wait_for_rounds(&stub, 3);
    assert_eq!(
        promoted,
        vec![
            vec!["first task".to_string()],
            vec!["first task".into(), "extra context".into()],
            vec!["first task".into(), "extra context".into(), "second task".into()],
        ],
        "round promotion: {promoted:?}"
    );

    // Post-completion retry of the same input id: same row, no new driver,
    // and after a settle window still no duplicate round.
    let (status, retry) = fleet.http("POST", &format!("/api/sessions/{SESSION}/prompt"), &queued);
    assert_eq!(status, 200, "queue retry: {retry}");
    assert_eq!(retry["admitted_seq"], json!(queued_seq), "same row: {retry}");
    assert_eq!(retry["driver_ensured"], json!(false), "no drain re-kick: {retry}");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(wait_for_rounds(&stub, 3).len(), 3, "no duplicate round");

    // A plain follow-up still drains normally with the composed history.
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/sessions/{SESSION}/prompt"),
        &json!({"prompt": "continue later", "input_id": "later"}),
    );
    assert_eq!(status, 200, "follow-up: {body}");
    fleet.wait_idle(SESSION);
    let later = wait_for_rounds(&stub, 4);
    assert_eq!(
        later.last(),
        Some(&vec![
            "first task".into(),
            "extra context".into(),
            "second task".into(),
            "continue later".into(),
        ]),
        "follow-up round: {later:?}"
    );
}
