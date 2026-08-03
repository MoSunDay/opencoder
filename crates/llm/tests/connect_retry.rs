//! Integration tests for the pre-stream `connect_with_retry` loop:
//!
//! - Bug 3: a retryable status (429) must drain its body and honor the
//!   `Retry-After` header (waiting at least the hinted number of seconds before
//!   the next attempt, rather than only the computed backoff).
//! - Bug 4: if the consumer drops the receiver mid-loop, the connect loop must
//!   stop issuing further requests instead of looping until the retry budget is
//!   exhausted.

use std::sync::atomic::{AtomicU32, Ordering};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use opencoder_llm::{ChatClient, ChatRequest, LlmEvent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Generous per-read window so the streaming portion of these tests is never
/// tripped by the idle watchdog (the connect loop itself is the subject here).
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// One scripted HTTP response served to a single accepted connection.
enum Resp {
    /// A plain (non-streaming) HTTP response with the given status, an optional
    /// `Retry-After` header (seconds), and a JSON-ish body. `Connection: close`
    /// forces a fresh TCP connection per attempt, so each accept is observable.
    Status {
        code: u16,
        retry_after: Option<u64>,
        body: String,
    },
    /// A fully healthy SSE stream (text + finish + [DONE]).
    Full { text: String },
}

/// Spawn a mock server that serves `behaviors` in FIFO order, counting every
/// accepted connection into `accepts`.
fn spawn_counted_server(
    listener: TcpListener,
    behaviors: VecDeque<Resp>,
    accepts: Arc<AtomicU32>,
) {
    let bq = Arc::new(Mutex::new(behaviors));
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => break,
            };
            accepts.fetch_add(1, Ordering::SeqCst);
            let beh = bq.lock().unwrap().pop_front();
            let beh = match beh {
                Some(b) => b,
                None => break,
            };
            tokio::spawn(async move {
                let _ = stream.set_nodelay(true);
                consume_http_request(&mut stream).await;
                match beh {
                    Resp::Status { code, retry_after, body } => {
                        let mut head = format!(
                            "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\n\
                             Content-Length: {len}\r\nConnection: close\r\n",
                            reason = http_reason(code),
                            len = body.len(),
                        );
                        if let Some(secs) = retry_after {
                            head.push_str(&format!("Retry-After: {secs}\r\n"));
                        }
                        head.push_str("\r\n");
                        let _ = stream.write_all(head.as_bytes()).await;
                        let _ = stream.write_all(body.as_bytes()).await;
                        let _ = stream.flush().await;
                    }
                    Resp::Full { text } => {
                        let _ = write_sse_header(&mut stream).await;
                        let _ = stream.write_all(sse_text(&text).as_bytes()).await;
                        let _ = stream.flush().await;
                        let _ = stream.write_all(sse_done().as_bytes()).await;
                        let _ = stream.flush().await;
                    }
                }
            });
        }
    });
}

async fn start_server(behaviors: VecDeque<Resp>, accepts: Arc<AtomicU32>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    spawn_counted_server(listener, behaviors, accepts);
    format!("http://{addr}")
}

fn make_client(base_url: &str) -> ChatClient {
    ChatClient::new_with_read_timeout(base_url, "test-key", &[], READ_TIMEOUT, None).unwrap()
}

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

async fn drain(rx: &mut tokio::sync::mpsc::Receiver<LlmEvent>) -> Vec<LlmEvent> {
    let mut out = Vec::new();
    while let Some(ev) = rx.recv().await {
        out.push(ev);
    }
    out
}

async fn consume_http_request(stream: &mut tokio::net::TcpStream) {
    let mut buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut buf).await.unwrap_or(0);
        if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
            return;
        }
    }
}

async fn write_sse_header(stream: &mut tokio::net::TcpStream) -> std::io::Result<()> {
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        )
        .await?;
    stream.flush().await
}

fn sse_text(content: &str) -> String {
    format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{content}\"}}}}]}}\n\n")
}

fn sse_done() -> &'static str {
    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
}

fn http_reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// A 429 carrying `Retry-After: 1` must delay the retry by ~1 s (the header's
/// hint), which is longer than the attempt-1 exponential backoff (~0.5 s) — so
/// honoring the header is observable as elapsed wall time. The body is also
/// drained so the connection can be reused.
#[tokio::test]
async fn retry_after_header_delays_retry_then_completes() {
    let url = start_server(
        VecDeque::from([
            Resp::Status {
                code: 429,
                retry_after: Some(1),
                body: r#"{"error":"rate limited"}"#.to_string(),
            },
            Resp::Full { text: "ok".to_string() },
        ]),
        Arc::new(AtomicU32::new(0)),
    )
    .await;

    let start = Instant::now();
    let mut rx = make_client(&url).chat_stream(make_request()).unwrap();
    let events = drain(&mut rx).await;
    let elapsed = start.elapsed();

    assert!(
        events
            .iter()
            .any(|e| matches!(e, LlmEvent::Retrying { attempt: 1, max: 5 })),
        "expected one connect-level retry: {events:?}"
    );
    match events.last() {
        Some(LlmEvent::Completed { text, .. }) => assert_eq!(text, "ok"),
        other => panic!("expected Completed, got {other:?}"),
    }
    // The header demands 1 s; pure backoff for attempt 1 is ~0.5 s. Requiring
    // >= 0.95 s proves the Retry-After hint was honored rather than ignored.
    assert!(
        elapsed >= Duration::from_millis(950),
        "Retry-After not honored (too fast): {elapsed:?}"
    );
    // Sanity upper bound so a regression that sleeps far too long is caught.
    assert!(elapsed < Duration::from_secs(6), "retry took too long: {elapsed:?}");
}

/// When the consumer drops the receiver mid connect-loop, the loop must notice
/// and stop. The server always returns a retryable 429 with a 2 s Retry-After,
/// so attempts are clearly spaced; without consumer-drop detection a second
/// request would fire ~2 s in.
#[tokio::test]
async fn consumer_drop_stops_connect_loop() {
    // Serve plenty of 429s so the server never runs dry during the window.
    let accepts = Arc::new(AtomicU32::new(0));
    let repeated = std::iter::repeat_with(|| Resp::Status {
        code: 429,
        retry_after: Some(2),
        body: "{}".to_string(),
    })
    .take(8)
    .collect::<VecDeque<_>>();
    let url = start_server(repeated, accepts.clone()).await;

    let mut rx = make_client(&url).chat_stream(make_request()).unwrap();
    // Block until the first 429 is processed and Retrying is emitted, so we are
    // guaranteed to be inside the connect retry loop before dropping the consumer.
    let first = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("timed out waiting for first Retrying event")
        .expect("channel closed before any event");
    assert!(
        matches!(first, LlmEvent::Retrying { .. }),
        "first event should be Retrying: {first:?}"
    );
    drop(rx); // consumer is gone

    // A second attempt, if attempted, would fire ~2 s after the first 429.
    // Wait past that window, then assert no second connection was made.
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let n = accepts.load(Ordering::SeqCst);
    assert!(
        n <= 1,
        "connect loop kept retrying after consumer dropped: {n} connections"
    );
}
