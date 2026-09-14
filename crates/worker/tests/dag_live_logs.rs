//! Real Node WebSocket -> control SSE, observed before the Wasm step exits.
mod support;
use futures::StreamExt;
use opencoder_llm::LlmEvent;
use serde_json::{json, Value};
use std::time::Duration;
use support::*;

fn ids(text: &str) -> Vec<i64> {
    text.lines()
        .filter_map(|line| line.strip_prefix("id: ")?.parse().ok())
        .collect()
}

#[tokio::test]
async fn dag_agent_and_wasm_logs_stream_before_completion_and_resume_by_sequence() {
    let _config = isolated_config();
    let client = mock();
    client.queue_script(vec![
        LlmEvent::TextDelta("agent-live-answer".into()),
        LlmEvent::Completed {
            text: "agent-live-answer".into(),
            tool_calls: vec![],
            usage: None,
        },
    ]);
    let fleet = Fleet::new(1, client).await;
    let modules = fleet.root().join("n0/node/dag/_modules");
    std::fs::create_dir_all(&modules).unwrap();
    let wasm = wat::parse_str(r#"(module
        (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 8) "wasm-live-output")
        (func (export "_start")
            (i32.store (i32.const 0) (i32.const 8))
            (i32.store (i32.const 4) (i32.const 16))
            (drop (call $write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 28)))
            (drop (call $write (i32.const 2) (i32.const 0) (i32.const 1) (i32.const 28)))
            (loop $spin (br $spin))))"#).unwrap();
    std::fs::write(modules.join("live.wasm"), wasm).unwrap();
    let saved = fleet.call("POST", "/api/dag/defs", json!({"spec":{"name":"live-logs","steps":[
        {"name":"answer","kind":{"type":"agent","prompt":"answer"}},
        {"name":"watch","depends_on":["answer"],"timeout_secs":30,"kind":{"type":"wasm","command":"live.wasm"}}
    ]}})).await;
    assert_eq!(saved.status, 200, "{saved:?}");
    let reply = fleet
        .call(
            "POST",
            "/api/dag/defs/live-logs/dispatch",
            json!({"id":"dag-live-logs"}),
        )
        .await;
    assert_eq!(reply.status, 202, "{reply:?}");
    let response = fleet
        .response("GET", "/api/executions/dag-live-logs/events?after=0")
        .await;
    assert_eq!(response.status(), 200);
    let mut stream = response.into_body().into_data_stream();
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        while !text.contains("\"event\":\"stderr\"") {
            let bytes = stream
                .next()
                .await
                .expect("live stream stays open")
                .unwrap();
            text.push_str(std::str::from_utf8(&bytes).unwrap());
        }
    })
    .await
    .expect("stdout and stderr must arrive while Wasm is still running");
    assert!(text.contains("agent-live-answer"), "{text}");
    assert!(text.contains("wasm-live-output"), "{text}");
    assert!(text.contains("\"event\":\"stdout\""), "{text}");
    assert!(!text.contains("event: run_finished"), "{text}");
    let detail = fleet
        .call("GET", "/api/executions/dag-live-logs", Value::Null)
        .await;
    assert_eq!(detail.body["execution"]["status"], "running", "{detail:?}");
    let cursor = *ids(&text).last().unwrap();
    drop(stream);
    let reply = fleet
        .call(
            "POST",
            "/api/executions/dag-live-logs/commands",
            json!({"action":"interrupt"}),
        )
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    let detail = settled(&fleet.nodes[0], "dag-live-logs").await;
    assert_eq!(detail["execution"]["status"], "interrupted", "{detail}");
    let response = fleet
        .response(
            "GET",
            &format!("/api/executions/dag-live-logs/events?after={cursor}"),
        )
        .await;
    let bytes = tokio::time::timeout(
        Duration::from_secs(10),
        axum::body::to_bytes(response.into_body(), 1024 * 1024),
    )
    .await
    .unwrap()
    .unwrap();
    let resumed = std::str::from_utf8(&bytes).unwrap();
    assert!(resumed.contains("event: run_finished"), "{resumed}");
    assert!(resumed.contains("event: stream_end"), "{resumed}");
    let sequences = ids(resumed);
    assert!(sequences.iter().all(|seq| *seq > cursor), "{sequences:?}");
    assert!(
        sequences.windows(2).all(|pair| pair[0] < pair[1]),
        "{sequences:?}"
    );
    fleet.shutdown().await;
}
