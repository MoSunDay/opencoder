//! SSE event streaming on `/api/executions/:id`: last-event-id header resume,
//! incremental tail polling, `more` paging, mid-stream error frames, and the
//! chunk-offset paging endpoints (event payload, detail field, messages).

use std::time::Duration;

use base64::Engine;
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::{json, Value};

use crate::support::{Harness, TOKEN};

/// SSE frame sequence ids in wire order (parses only `id: <n>` lines).
fn sse_ids(text: &str) -> Vec<i64> {
    text.lines()
        .filter_map(|line| line.strip_prefix("id: "))
        .filter_map(|value| value.parse().ok())
        .collect()
}

fn row(seq: i64, kind: &str) -> Value {
    json!({"seq": seq, "kind": kind, "data": {"n": seq}, "ts": seq})
}

fn b64(text: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(text)
}

fn decode_b64(chunk: &Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(chunk["bytes_b64"].as_str().unwrap())
        .unwrap()
}

#[tokio::test]
async fn last_event_id_header_resumes_after_the_named_seq() {
    let h = Harness::new().await;
    h.put_index("agent-hdr-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node.set_events(
        "agent-hdr-1",
        vec![
            row(1, "llm_round_start"),
            row(2, "text_delta"),
            row(3, "done"),
        ],
        true,
    );
    let (status, bytes) = h
        .req_bytes(
            Method::GET,
            "/api/executions/agent-hdr-1/events",
            None,
            None,
            Some(TOKEN),
            &[("last-event-id", "1")],
        )
        .await;
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(status, 200, "{text}");
    // Only seq 2 and 3 replay; seq 1 is behind the resume cursor.
    assert_eq!(sse_ids(&text), vec![2, 3], "{text}");
    assert!(
        text.contains("event: text_delta") && text.contains("event: done"),
        "{text}"
    );
}

#[tokio::test]
async fn incremental_tail_emits_late_rows_and_closes_on_finished() {
    let h = Harness::new().await;
    h.put_index(
        "agent-tail-1",
        ExecutionKind::Agent,
        ExecutionStatus::Running,
    )
    .await;
    let first = vec![row(1, "llm_round_start"), row(2, "text_delta")];
    h.node.set_events("agent-tail-1", first.clone(), false);
    let stream = {
        let h = h.clone();
        tokio::spawn(async move { h.sse_text("/api/executions/agent-tail-1/events").await })
    };
    // The unfinished stream stays open while polling; new rows appear later.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut all = first;
    all.push(row(3, "tool_result"));
    all.push(row(4, "done"));
    h.node.set_events("agent-tail-1", all, true);
    let (status, text) = tokio::time::timeout(Duration::from_secs(10), stream)
        .await
        .expect("stream closes once the node reports finished")
        .unwrap();
    assert_eq!(status, 200);
    assert_eq!(sse_ids(&text), vec![1, 2, 3, 4], "{text}");
}

#[tokio::test]
async fn more_flag_keeps_polling_and_pages_without_duplicates() {
    let h = Harness::new().await;
    h.put_index(
        "agent-more-1",
        ExecutionKind::Agent,
        ExecutionStatus::Running,
    )
    .await;
    let first = vec![row(1, "llm_round_start"), row(2, "text_delta")];
    // finished=false + more=true: the first page replays but polling goes on.
    h.node
        .set_events_more("agent-more-1", first.clone(), false, true);
    let stream = {
        let h = h.clone();
        tokio::spawn(async move { h.sse_text("/api/executions/agent-more-1/events").await })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut all = first;
    all.push(row(3, "text_delta"));
    all.push(row(4, "done"));
    // Flip more off with a finished page holding the second batch.
    h.node.set_events_more("agent-more-1", all, true, false);
    let (status, text) = tokio::time::timeout(Duration::from_secs(10), stream)
        .await
        .expect("stream closes once more flips off with finished")
        .unwrap();
    assert_eq!(status, 200);
    // Re-served rows are dropped by the cursor: every seq appears once.
    assert_eq!(sse_ids(&text), vec![1, 2, 3, 4], "{text}");
}

#[tokio::test]
async fn mid_stream_node_error_emits_an_error_frame_and_closes() {
    let h = Harness::new().await;
    h.put_index(
        "agent-err-1",
        ExecutionKind::Agent,
        ExecutionStatus::Running,
    )
    .await;
    h.node
        .set_events("agent-err-1", vec![row(1, "llm_round_start")], false);
    let stream = {
        let h = h.clone();
        tokio::spawn(async move { h.sse_text("/api/executions/agent-err-1/events").await })
    };
    // Let the first page flush, then make the node fail the next poll.
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.node
        .set_events_status("agent-err-1", 500, json!({"error": "boom"}));
    let (status, text) = tokio::time::timeout(Duration::from_secs(10), stream)
        .await
        .expect("error frame terminates the stream")
        .unwrap();
    assert_eq!(status, 200);
    assert!(text.contains("event: error"), "{text}");
    assert!(text.contains(r#"data: {"error":"boom"}"#), "{text}");
    // The replayed row stays framed; the error frame carries no id.
    assert_eq!(sse_ids(&text), vec![1], "{text}");
}

#[tokio::test]
async fn event_payload_chunks_reassemble_across_offset_pages() {
    let h = Harness::new().await;
    h.put_index("agent-pl-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    // First page: half the payload, eof=false with an explicit next_offset.
    h.node.set_payload(
        "agent-pl-1",
        7,
        json!({"seq": 7, "offset": 0, "next_offset": 3, "total_bytes": 5,
               "eof": false, "encoding": "base64", "bytes_b64": b64("hel")}),
    );
    let (status, first) = h
        .req(
            Method::GET,
            "/api/executions/agent-pl-1/events/7/payload",
            None,
        )
        .await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["eof"], json!(false));
    assert_eq!(first["next_offset"], json!(3));
    // The scripted node keys payload replies by (id, seq), so the follow-up
    // page is re-seeded before chasing next_offset.
    h.node.set_payload(
        "agent-pl-1",
        7,
        json!({"seq": 7, "offset": 3, "next_offset": 5, "total_bytes": 5,
               "eof": true, "encoding": "base64", "bytes_b64": b64("lo")}),
    );
    let (status, second) = h
        .req(
            Method::GET,
            "/api/executions/agent-pl-1/events/7/payload?offset=3",
            None,
        )
        .await;
    assert_eq!(status, 200, "{second}");
    assert_eq!(second["offset"], json!(3));
    assert_eq!(second["eof"], json!(true));
    let mut bytes = decode_b64(&first);
    bytes.extend(decode_b64(&second));
    assert_eq!(bytes, b"hello");
}

#[tokio::test]
async fn detail_field_chunks_reassemble_across_offset_pages() {
    let h = Harness::new().await;
    h.put_index("agent-df-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node.set_field(
        "agent-df-1",
        "request.input",
        json!({"field": "request.input", "offset": 0, "next_offset": 4,
               "total_bytes": 8, "eof": false, "encoding": "json-base64",
               "bytes_b64": b64("{\"a\"")}),
    );
    let (status, first) = h
        .req(
            Method::GET,
            "/api/executions/agent-df-1/detail-field?field=request.input",
            None,
        )
        .await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["field"], json!("request.input"));
    assert_eq!(first["next_offset"], json!(4));
    h.node.set_field(
        "agent-df-1",
        "request.input",
        json!({"field": "request.input", "offset": 4, "next_offset": 8,
               "total_bytes": 8, "eof": true, "encoding": "json-base64",
               "bytes_b64": b64(":true}")}),
    );
    let (status, second) = h
        .req(
            Method::GET,
            "/api/executions/agent-df-1/detail-field?field=request.input&offset=4",
            None,
        )
        .await;
    assert_eq!(status, 200, "{second}");
    assert_eq!(second["offset"], json!(4));
    assert_eq!(second["eof"], json!(true));
    let mut bytes = decode_b64(&first);
    bytes.extend(decode_b64(&second));
    assert_eq!(bytes, br#"{"a":true}"#);
}

#[tokio::test]
async fn messages_chunk_paging_follows_the_next_cursor() {
    let h = Harness::new().await;
    h.put_index("agent-msg-2", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node.set_messages(
        "agent-msg-2",
        json!({
            "chunks": [{"seq": 1, "role": "user", "offset": 0, "next_offset": 4,
                        "total_bytes": 8, "eof": false, "encoding": "base64",
                        "bytes_b64": b64("user")}],
            "next_cursor": {"seq": 1, "offset": 4},
            "more": true,
        }),
    );
    let (status, first) = h
        .req(Method::GET, "/api/executions/agent-msg-2/messages", None)
        .await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["more"], json!(true));
    assert_eq!(first["next_cursor"], json!({"seq": 1, "offset": 4}));
    // Follow the advertised cursor: the tail chunk closes the message.
    h.node.set_messages(
        "agent-msg-2",
        json!({
            "chunks": [{"seq": 1, "role": "user", "offset": 4, "next_offset": 8,
                        "total_bytes": 8, "eof": true, "encoding": "base64",
                        "bytes_b64": b64("name")}],
            "next_cursor": null,
            "more": false,
        }),
    );
    let (status, second) = h
        .req(
            Method::GET,
            "/api/executions/agent-msg-2/messages?seq=1&offset=4",
            None,
        )
        .await;
    assert_eq!(status, 200, "{second}");
    assert_eq!(second["more"], json!(false));
    let mut bytes = decode_b64(&first["chunks"][0]);
    bytes.extend(decode_b64(&second["chunks"][0]));
    assert_eq!(bytes, b"username");
}
