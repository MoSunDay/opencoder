//! Integration tests for per-chunk read timeout behavior on `ChatClient`.
//!
//! These tests spin up a minimal mock SSE server on a TCP socket (no extra
//! dependencies) and verify two key properties of the `read_timeout` setting:
//!
//! 1. A stream that delivers data continuously is **not** interrupted, even
//!    when the total duration exceeds the per-read timeout (each chunk resets
//!    the timer).
//! 2. A stream that stalls (sends no data) **is** interrupted by the
//!    `read_timeout`, producing an `LlmEvent::Error` far sooner than any
//!    absolute timeout would.

use std::time::{Duration, Instant};

use opencoder_llm::{ChatClient, ChatRequest, LlmEvent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Build a minimal `ChatRequest` (content doesn't matter for these tests).
fn make_request() -> ChatRequest {
    ChatRequest {
        model: "test-model".to_string(),
        messages: vec![serde_json::json!({"role": "user", "content": "hi"})],
        tools: vec![],
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: None,
        cache_salt: None,
    }
}

/// Collect all events from a receiver until the channel closes.
async fn drain(rx: &mut tokio::sync::mpsc::Receiver<LlmEvent>) -> Vec<LlmEvent> {
    let mut out = Vec::new();
    while let Some(ev) = rx.recv().await {
        out.push(ev);
    }
    out
}

/// Read and discard the HTTP request line + headers from the client. Stops
/// after consuming the blank line (`\r\n\r\n`) that terminates the header
/// section. The request body is left unread — it is small enough to fit in
/// the TCP receive buffer and the client is already waiting for a response.
async fn consume_http_request(stream: &mut tokio::net::TcpStream) {
    let mut buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
            return;
        }
    }
}

/// Write the HTTP response header for an SSE stream.
async fn write_sse_header(stream: &mut tokio::net::TcpStream) {
    let header = "HTTP/1.1 200 OK\r\n\
                  Content-Type: text/event-stream\r\n\
                  Cache-Control: no-cache\r\n\
                  Connection: close\r\n\
                  \r\n";
    stream.write_all(header.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();
}

/// Format a text-delta SSE chunk.
fn sse_text(content: &str) -> String {
    format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{content}\"}}}}]}}\n\n")
}

/// Format a finish SSE chunk + [DONE] marker.
fn sse_done() -> &'static str {
    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
}

/// Test 1 — A slow but continuously delivering stream must NOT be interrupted
/// by `read_timeout`, even when the total stream duration exceeds it.
///
/// We send 25 chunks at 50 ms intervals (total ~1.25 s) with a 1 s
/// `read_timeout`. Under the old absolute `.timeout()` the stream would be
/// killed at 1.0 s; under per-read `read_timeout` each chunk resets the
/// timer and the stream completes normally.
#[tokio::test]
async fn continuous_stream_not_interrupted_by_read_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let chunk_count: u32 = 25;
    let chunk_interval = Duration::from_millis(50);
    let read_timeout = Duration::from_secs(1);

    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.set_nodelay(true).unwrap();
        consume_http_request(&mut stream).await;
        write_sse_header(&mut stream).await;

        for i in 0..chunk_count {
            let chunk = sse_text(&format!("c{i}"));
            stream.write_all(chunk.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            tokio::time::sleep(chunk_interval).await;
        }
        stream.write_all(sse_done().as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        // Dropping `stream` closes the connection -> stream ends -> Completed.
    });

    let client =
        ChatClient::new_with_read_timeout(&base_url, "test-key", &[], read_timeout, None).unwrap();
    let mut rx = client.chat_stream(make_request()).unwrap();

    let events = drain(&mut rx).await;

    // Collect text deltas.
    let texts: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            LlmEvent::TextDelta(t) => Some(t.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(texts.len(), chunk_count as usize, "all chunks must arrive");
    assert_eq!(texts[0], "c0");
    assert_eq!(texts[chunk_count as usize - 1], "c24");

    // Must end with Completed, not Error.
    let completed = events
        .iter()
        .any(|e| matches!(e, LlmEvent::Completed { .. }));
    let has_error = events.iter().any(|e| matches!(e, LlmEvent::Error(_)));
    assert!(
        completed,
        "stream should complete, got: {:?}",
        events.last()
    );
    assert!(!has_error, "no error expected for a healthy stream");
}

/// Test 2 — A stalled stream (no data after the first chunk) must be
/// interrupted by `read_timeout`, producing `LlmEvent::Error` quickly.
///
/// The server sends one chunk then hangs indefinitely. With `read_timeout`
/// set to 500 ms the client should abort well under 5 s — proving the
/// timeout is per-read (idle) based, not a long absolute deadline.
#[tokio::test]
async fn stalled_stream_interrupted_by_read_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let read_timeout = Duration::from_millis(500);

    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.set_nodelay(true).unwrap();
        consume_http_request(&mut stream).await;
        write_sse_header(&mut stream).await;

        // Send one chunk, then stall forever.
        stream.write_all(sse_text("only").as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        // Sleep for a very long time — longer than any reasonable test timeout.
        tokio::time::sleep(Duration::from_secs(120)).await;
    });

    let client =
        ChatClient::new_with_read_timeout(&base_url, "test-key", &[], read_timeout, None).unwrap();
    let mut rx = client.chat_stream(make_request()).unwrap();

    let start = Instant::now();
    let events = drain(&mut rx).await;
    let elapsed = start.elapsed();

    // At least one text delta should have arrived before the stall.
    let has_text = events
        .iter()
        .any(|e| matches!(e, LlmEvent::TextDelta(t) if t == "only"));
    assert!(
        has_text,
        "first chunk should arrive before stall: {:?}",
        events
    );

    // An Error event must be present (read timeout -> stream failure).
    let has_error = events
        .iter()
        .any(|e| matches!(e, LlmEvent::Error(msg) if msg.contains("stream failed")));
    assert!(
        has_error,
        "expected Error from read timeout, got: {:?}",
        events.last()
    );

    // No Completed event — the stream was aborted, not finished.
    let has_completed = events
        .iter()
        .any(|e| matches!(e, LlmEvent::Completed { .. }));
    assert!(!has_completed, "stalled stream must not produce Completed");

    // Must complete well under the old 1800 s absolute timeout.
    assert!(
        elapsed < Duration::from_secs(5),
        "read_timeout should abort in ~500 ms, took {elapsed:?}"
    );
}
