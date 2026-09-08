#![cfg(unix)]
#[path = "../../session/tests/harness/binary.rs"]
mod binary;
mod support;
use base64::Engine;
use serde_json::{json, Value};
use support::*;

#[tokio::test]
async fn server_dispatches_codex_to_node_and_replays_native_messages() {
    let client = mock();
    let fleet = Fleet::new(1, client.clone()).await;
    let bin = binary::fake_binary(fleet.root());
    let capture = fleet.root().join("capture.jsonl");
    let input = json!({"id":"agent-codex-test","kind":"agent","target":"act","input":{
        "harness":"codex","prompt":"first request","envs":{
            "PATH":format!("{}:/usr/bin:/bin",bin.display()),"CAPTURE":capture,
            "EXAMPLE":"private-launch-value"
        }
    }});
    let accepted = fleet.call("POST", "/api/executions", input.clone()).await;
    assert_eq!(accepted.status, 202, "{accepted:?}");
    let detail = settled(&fleet.nodes[0], "agent-codex-test").await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    assert_eq!(detail["session"]["harness"], "codex");
    assert!(!detail.to_string().contains("private-launch-value"));
    assert_eq!(client.call_count(), 0, "Codex must not use native provider");
    let retry = fleet.call("POST", "/api/executions", input).await;
    assert_eq!(retry.status, 202);
    assert_eq!(
        std::fs::read_to_string(&capture).unwrap().lines().count(),
        1
    );
    let prompt = fleet
        .call(
            "POST",
            "/api/executions/agent-codex-test/commands",
            json!({"action":"prompt","input":{"prompt":"next request"}}),
        )
        .await;
    assert!(prompt.status < 300, "{prompt:?}");
    let detail = settled(&fleet.nodes[0], "agent-codex-test").await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    let messages = fleet
        .call(
            "GET",
            "/api/executions/agent-codex-test/messages",
            Value::Null,
        )
        .await;
    assert_eq!(messages.status, 200, "{messages:?}");
    let transcript: String = messages.body["chunks"]
        .as_array()
        .expect("message chunks")
        .iter()
        .map(|chunk| {
            String::from_utf8(
                base64::engine::general_purpose::STANDARD
                    .decode(chunk["bytes_b64"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap()
        })
        .collect();
    assert!(transcript.contains("tool result"), "{transcript}");
    let records: Vec<Value> = std::fs::read_to_string(capture)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1]["args"][1], "resume");
    assert_eq!(records[1]["prompt"], "next request");
    let invalid = fleet.call("POST", "/api/executions", json!({"id":"agent-invalid-harness","kind":"agent","target":"act","input":{"harness":"unknown","prompt":"x"}})).await;
    assert!(invalid.status >= 400);
    fleet.shutdown().await;
}
