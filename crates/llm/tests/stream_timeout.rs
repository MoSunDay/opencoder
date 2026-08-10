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

use std::time::Duration;

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
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
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

/// A stream that goes "keep-alive only" mid-response must be interrupted by
/// the event-level idle watchdog, producing an `LlmEvent::Error` — not
/// hanging until the HTTP layer's own timeout (600 s by default).
///
/// After delivering one data chunk, the upstream keeps the connection alive
/// with SSE comment frames (`: keep-alive\n\n`) but sends no more data. Bytes
/// keep flowing (so reqwest's byte-level `read_timeout` never trips), yet no
/// decoded SSE *data* event arrives — exactly the stall the `IdleTimeout`
/// guard in `run_stream_once` exists to catch.
///
/// This regression-tests the idle-timeout watchdog (`last_event_at`) plus the
/// `tokio::time::timeout(idle_timeout, stream.next())` wrapper: a
/// keep-alive-only upstream can never hold the consumer hostage.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_stream_interrupted_by_idle_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    // Match the healthy-stream test's stable scheduling margin. The heartbeat
    // cadence remains well inside this window, so this still isolates the
    // event-level idle watchdog from reqwest's byte-level read timeout.
    let read_timeout = Duration::from_secs(1);

    tokio::spawn(async move {
        // The stream retry loop reconnects up to MAX_STREAM_ATTEMPTS (3) times
        // when an idle timeout fires, so accept that many connections. Each
        // one is serviced in its own task because every handler holds its
        // connection open (stalled) while the next reconnect is accepted.
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().await.unwrap();
            stream.set_nodelay(true).unwrap();
            tokio::spawn(async move {
                consume_http_request(&mut stream).await;
                write_sse_header(&mut stream).await;

                // Send one data frame, then switch to keep-alive-only.
                stream
                    .write_all(sse_text("partial").as_bytes())
                    .await
                    .unwrap();
                stream.flush().await.unwrap();

                // SSE comment frames (`: keep-alive\n\n`) carry no `data:`
                // line, so the decoder yields nothing — yet the bytes keep
                // reqwest's byte-level read_timeout satisfied. With no decoded
                // data event arriving, the event-level idle watchdog in
                // `run_stream_once` trips (IdleTimeout), not the byte-level
                // timeout (ChunkError). Heartbeats every 100 ms — well inside
                // the 1 s read window — mirror a real keep-alive-only stall;
                // ~3 s comfortably outlasts one attempt's idle window.
                for _ in 0..30 {
                    // `let _ =`: the client drops this connection when it
                    // retries after the idle timeout, so later writes hit a
                    // broken pipe — ignore them and let the loop wind down.
                    let _ = stream.write_all(b": keep-alive\n\n").await;
                    let _ = stream.flush().await;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            });
        }
        // Keep the listener bound while the spawned handlers run.
        tokio::time::sleep(Duration::from_secs(30)).await;
    });

    let client =
        ChatClient::new_with_read_timeout(&base_url, "test-key", &[], read_timeout, None).unwrap();
    let mut rx = client.chat_stream(make_request()).unwrap();

    let events = drain(&mut rx).await;

    // Must contain at least the partial text delta.
    let has_text = events
        .iter()
        .any(|e| matches!(e, LlmEvent::TextDelta(t) if t == "partial"));
    assert!(has_text, "partial text must arrive before stall");

    // Must end with an Error (idle timeout after retries), NOT hang.
    let has_error = events
        .iter()
        .any(|e| matches!(e, LlmEvent::Error(msg) if msg.contains("idle timeout")));
    assert!(
        has_error,
        "stalled stream must be interrupted by idle timeout, got: {:?}",
        events.last()
    );
}
