//! O1 — the full operator face: `POST /api/sessions` creates the session
//! AND its operator execution in one call; the node runs the prompt
//! through the real agent loop against the LLM stub; the execution folds
//! to `idle` (the operator's success state), the inspect document exposes
//! the session pointer and final answer as the result, and the decoded transcript (via the
//! native session GET) contains both sides of the exchange.

use crate::support::fleet_proc::{Fleet, TOKEN};
use crate::support::http_util::sse_read;
use crate::support::llm_stub::LlmStub;
use serde_json::json;

const PROMPT: &str = "e2e-operator-prompt: 报告节点状态";
const REPLY: &str = "e2e-operator-reply";
const SESSION: &str = "operator-e2e-flow-1";

#[test]
fn operator_session_runs_prompt_to_idle() {
    let stub = LlmStub::spawn_text(&[REPLY, "follow-up reply"]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "op-flow-node");

    // One call creates the session and its operator-kind execution.
    let (status, body) = fleet.http(
        "POST",
        "/api/sessions",
        &json!({"id": SESSION, "node_id": fleet.node_id(), "agent": "act", "prompt": PROMPT}),
    );
    assert_eq!(status, 200, "create session: {body}");
    assert_eq!(body["id"], SESSION);
    assert_eq!(body["execution"]["kind"], "operator");
    assert_eq!(body["execution"]["node_id"], fleet.node_id());

    // The operator's success state is `idle` (a live chat session, NOT a
    // terminal status).
    let doc = fleet.wait_idle(SESSION);
    assert_eq!(doc["execution"]["status"], "idle", "inspect: {doc}");
    assert_eq!(doc["execution"]["kind"], "operator");
    // The result exposes the answer to consumers such as project TODOs.
    assert_eq!(
        doc["result"],
        json!({"session_id": SESSION, "output_text": REPLY, "output_json": null})
    );
    // The session projection carries the Operator title.
    assert_eq!(doc["session"]["meta"]["id"], SESSION);
    assert_eq!(doc["session"]["meta"]["title"], "Operator");

    // The decoded transcript (native session GET through the relay) has
    // the unchanged user prompt and the stub's reply.
    let (status, detail) = fleet.http("GET", &format!("/api/sessions/{SESSION}"), &json!({}));
    assert_eq!(status, 200, "session detail: {detail}");
    assert_eq!(detail["id"], SESSION);
    assert_eq!(detail["draining"], json!(false), "drain finished");
    let transcript = detail["messages"].to_string();
    assert!(transcript.contains(PROMPT), "transcript: {transcript}");
    assert!(
        transcript.contains(REPLY),
        "reply must land in the transcript"
    );
    assert!(!transcript.contains("Operator 默认规则"));

    // Mode defaults belong in the system message, outside the transcript.
    let requests = stub.wait_for_requests(1);
    assert!(
        requests[0].contains(PROMPT),
        "prompt hit the LLM: {}",
        requests[0]
    );
    let request: serde_json::Value = serde_json::from_str(&requests[0]).unwrap();
    let system = request["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("Operator 默认规则"));
    assert!(system.contains("每次调用聚焦一个可验证的结果"));
    let skills = fleet
        .node_data
        .join("operator")
        .join(SESSION)
        .join("home/.opencoder/skills")
        .canonicalize()
        .unwrap();
    assert!(skills.join("task-plan/SKILL.md").is_file());
    assert!(system.contains(skills.to_str().unwrap()));

    // The completed stream replays: text_delta frames carry the reply and
    // the stream ends with the terminal `stream_end` marker.
    let frames = sse_read(
        &fleet.base,
        &format!("/api/sessions/{SESSION}/events"),
        TOKEN,
        0,
        None,
    );
    assert!(!frames.is_empty(), "no frames");
    let deltas = frames
        .iter()
        .filter(|f| f.event == "text_delta")
        .map(|f| f.data["text"].as_str().unwrap_or_default())
        .collect::<String>();
    assert!(deltas.contains(REPLY), "deltas: {deltas}");
    assert_eq!(frames.last().unwrap().event, "stream_end");
    assert_eq!(frames.last().unwrap().data["finished"], json!(true));

    let (status, body) = fleet.http(
        "POST",
        &format!("/api/sessions/{SESSION}/prompt"),
        &json!({"prompt":"verify again", "input_id":"operator-followup"}),
    );
    assert!(status < 300, "{body}");
    let requests = stub.wait_for_requests(2);
    let request: serde_json::Value = serde_json::from_str(&requests[1]).unwrap();
    let system = request["messages"][0]["content"].as_str().unwrap();
    assert_eq!(system.matches("## Operator 默认规则").count(), 1);
    assert!(system.contains(skills.to_str().unwrap()));
    assert_eq!(
        fleet.wait_idle(SESSION)["result"]["output_text"],
        "follow-up reply"
    );
}
