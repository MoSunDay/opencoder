//! Integration tests for mid-stream retry on `ChatClient`.
//!
//! Each test spins up a mock SSE server on a TCP socket (no extra deps) whose
//! per-connection behavior is scripted via a [`Conn`] queue. This exercises the
//! real HTTP streaming + retry path end to end: an interrupted first attempt
//! must be discarded and regenerated, and the final `Completed.text` must
//! always come from a single healthy frame — never stitched across attempts.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use opencoder_llm::{ChatClient, ChatRequest, LlmEvent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Keep enough scheduling margin for a full-workspace test run while remaining
/// short enough for the retry scenarios to finish promptly.
const READ_TIMEOUT: Duration = Duration::from_secs(1);
const STALL_HOLD: Duration = Duration::from_secs(2);

/// What one accepted connection does before it is abandoned/finished.
enum Conn {
    /// Send `delta`, then hold the connection open (no more bytes) longer than
    /// the read window. A per-read timeout surfaces this as a chunk error.
    Stall { delta: String, hold: Duration },
    /// Serve the body with chunked framing: `delta` as one valid chunk, then
    /// a malformed chunk-size line. The client's read fails at the malformed
    /// line — a deterministic chunk error with no timer involved. Unlike
    /// `Stall`, whose silence arms the byte-level read timeout and the
    /// event-level idle watchdog at the same instant (this suite's
    /// constructor ties both to 1s, so under load the interruption kind
    /// flips to `idle timeout` — observed flake).
    Reset { delta: String },
    /// Send `delta`, then close cleanly WITHOUT a `finish_reason`. Surfaces as
    /// a truncated stream.
    Truncate { delta: String },
    /// Emit keep-alive heartbeats (`: ping`) — bytes flow but no data frames —
    /// for `hold`, longer than the event-level idle window. Surfaces as an
    /// idle-timeout interruption.
    Heartbeat { hold: Duration },
    /// A fully healthy stream: `delta` text + finish + `[DONE]`.
    Full { text: String },
}

/// Spawn a scripted mock SSE server. Connections are served in FIFO order from
/// the `behaviors` queue; each gets its own task so a stalling connection never
/// blocks the next retry's accept.
fn spawn_server(listener: TcpListener, behaviors: Vec<Conn>) {
    let bq = Arc::new(Mutex::new(VecDeque::<Conn>::from(behaviors)));
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => break,
            };
            let beh = bq.lock().unwrap().pop_front();
            let beh = match beh {
                Some(b) => b,
                None => break,
            };
            tokio::spawn(async move {
                let _ = stream.set_nodelay(true);
                consume_http_request(&mut stream).await;
                match beh {
                    Conn::Stall { delta, hold } => {
                        let _ = write_sse_header(&mut stream).await;
                        let _ = stream.write_all(sse_text(&delta).as_bytes()).await;
                        let _ = stream.flush().await;
                        tokio::time::sleep(hold).await;
                    }
                    Conn::Reset { delta } => {
                        // Chunked framing lets the arm inject a malformed
                        // chunk-size line — hyper's body read then errors
                        // deterministically (a clean FIN would be Truncated;
                        // a silent hold would race the idle watchdog).
                        let _ = write_sse_header_chunked(&mut stream).await;
                        let frame = sse_text(&delta);
                        let _ = stream
                            .write_all(format!("{:x}\r\n{frame}\r\n", frame.len()).as_bytes())
                            .await;
                        let _ = stream.flush().await;
                        // Not a hex length: the read of this line must fail.
                        let _ = stream.write_all(b"not-a-chunk-size\r\n").await;
                        let _ = stream.flush().await;
                    }
                    Conn::Truncate { delta } => {
                        let _ = write_sse_header(&mut stream).await;
                        let _ = stream.write_all(sse_text(&delta).as_bytes()).await;
                        let _ = stream.flush().await;
                        // Drop -> clean EOF, no finish_reason.
                    }
                    Conn::Heartbeat { hold } => {
                        let _ = write_sse_header(&mut stream).await;
                        let end = Instant::now() + hold;
                        while Instant::now() < end {
                            let _ = stream.write_all(b": ping\n\n").await;
                            let _ = stream.flush().await;
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                    Conn::Full { text } => {
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

/// Bind a server with the given scripted connections, returning its base URL.
async fn start_server(behaviors: Vec<Conn>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    spawn_server(listener, behaviors);
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

/// Chunked-transfer variant used by `Conn::Reset`: the body is chunk-framed
/// so a malformed chunk-size line can fail the client's body read on cue.
async fn write_sse_header_chunked(stream: &mut tokio::net::TcpStream) -> std::io::Result<()> {
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nTransfer-Encoding: chunked\r\n\r\n",
        )
        .await?;
    stream.flush().await
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

/// A stream truncated mid-way (clean EOF, no finish_reason) is retried; the
/// retried attempt delivers a complete frame and the persisted text is the
/// FINAL frame's text alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn truncated_stream_retries_then_completes() {
    let url = start_server(vec![
        Conn::Truncate {
            delta: "partial".into(),
        },
        Conn::Full {
            text: "final".into(),
        },
    ])
    .await;
    let mut rx = make_client(&url).chat_stream(make_request()).unwrap();
    let events = drain(&mut rx).await;

    let retry_count = events
        .iter()
        .filter(|e| matches!(e, LlmEvent::Retrying { .. }))
        .count();
    assert_eq!(retry_count, 1, "exactly one retry: {events:?}");

    match events.last() {
        Some(LlmEvent::Completed { text, .. }) => assert_eq!(text, "final"),
        other => panic!("expected Completed with final text, got {other:?}"),
    }
}

/// A connection reset/stall mid-stream (per-read timeout -> chunk error) is
/// retried and recovers on the second attempt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chunk_error_retries_then_completes() {
    let url = start_server(vec![
        Conn::Stall {
            delta: "partial".into(),
            hold: STALL_HOLD,
        },
        Conn::Full {
            text: "recovered".into(),
        },
    ])
    .await;
    let start = Instant::now();
    let mut rx = make_client(&url).chat_stream(make_request()).unwrap();
    let events = drain(&mut rx).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, LlmEvent::Retrying { attempt: 1, max: 3 })),
        "expected Retrying 1/3: {events:?}"
    );
    match events.last() {
        Some(LlmEvent::Completed { text, .. }) => assert_eq!(text, "recovered"),
        other => panic!("expected Completed, got {other:?}"),
    }
    // First attempt stalls ~1s, backoff ~0.5s, second attempt is immediate.
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "{:?}",
        start.elapsed()
    );
}

/// A connection that dribbles keep-alive heartbeats but no content triggers
/// the event-level idle watchdog, is retried, and recovers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_heartbeat_retries_then_completes() {
    let url = start_server(vec![
        Conn::Heartbeat { hold: STALL_HOLD },
        Conn::Full {
            text: "after-idle".into(),
        },
    ])
    .await;
    let mut rx = make_client(&url).chat_stream(make_request()).unwrap();
    let events = drain(&mut rx).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, LlmEvent::Retrying { .. })),
        "heartbeat-only stream should trigger a retry: {events:?}"
    );
    match events.last() {
        Some(LlmEvent::Completed { text, .. }) => assert_eq!(text, "after-idle"),
        other => panic!("expected Completed, got {other:?}"),
    }
}

/// When every attempt is interrupted, the budget is exhausted and a terminal
/// `Error` is emitted (no `Completed`). The `Retrying` events carry the right
/// attempt/max sequence.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_exhaustion_emits_error() {
    let url = start_server(vec![
        Conn::Stall {
            delta: "a".into(),
            hold: STALL_HOLD,
        },
        Conn::Stall {
            delta: "b".into(),
            hold: STALL_HOLD,
        },
        Conn::Stall {
            delta: "c".into(),
            hold: STALL_HOLD,
        },
    ])
    .await;
    let start = Instant::now();
    let mut rx = make_client(&url).chat_stream(make_request()).unwrap();
    let events = drain(&mut rx).await;

    let attempts: Vec<u8> = events
        .iter()
        .filter_map(|e| match e {
            LlmEvent::Retrying { attempt, max } if *max == 3 => Some(*attempt),
            _ => None,
        })
        .collect();
    assert_eq!(attempts, vec![1, 2], "retry sequence: {events:?}");

    assert!(
        events
            .iter()
            .any(|e| matches!(e, LlmEvent::Error(msg) if msg.contains("stream failed"))),
        "expected terminal Error: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, LlmEvent::Completed { .. })),
        "exhausted stream must not Completed: {events:?}"
    );
    // 3 stalls (~1s each) + backoffs (0.5s + 1s).
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "{:?}",
        start.elapsed()
    );
}

/// Regression (Bug 4): on the exhaustion path the consumer must receive
/// EXACTLY ONE `LlmEvent::Error`. Previously the `ChunkError`/`IdleTimeout`
/// arm both sent an `LlmEvent::Error` via `tx` *and* returned `Err`, and the
/// `chat_stream` spawn wrapper re-wrapped the `Err` into a second
/// `LlmEvent::Error` — so consumers saw two errors, the second with a doubled
/// `"stream failed: stream failed: ..."` prefix.
///
/// Uses `Conn::Reset` so the interruption kind is deterministically
/// `chunk read error`: with `Stall`, the byte-level read timeout and the idle
/// watchdog are armed at the same instant with the same 1s value, and under a
/// full-workspace test run whichever fired first flipped the kind to
/// `idle timeout` (observed flake).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_exhaustion_emits_single_non_doubled_error() {
    let url = start_server(vec![
        Conn::Reset { delta: "a".into() },
        Conn::Reset { delta: "b".into() },
        Conn::Reset { delta: "c".into() },
    ])
    .await;
    let mut rx = make_client(&url).chat_stream(make_request()).unwrap();
    let events = drain(&mut rx).await;

    let errors: Vec<&String> = events
        .iter()
        .filter_map(|e| match e {
            LlmEvent::Error(msg) => Some(msg),
            _ => None,
        })
        .collect();

    // Exactly one terminal error — never two.
    assert_eq!(
        errors.len(),
        1,
        "exhaustion must emit exactly one Error, got {errors:?} (events: {events:?})"
    );

    let msg = errors[0];
    // The wrapper adds a single "stream failed: " prefix; the inner Err no
    // longer carries its own prefix, so the text must not be doubled.
    assert!(
        msg.contains("stream failed"),
        "single error must carry the wrapper prefix: {msg:?}"
    );
    assert!(
        !msg.contains("stream failed: stream failed"),
        "error message must NOT be double-prefixed: {msg:?}"
    );
    // The kind label and attempt count survive the single-prefix path.
    assert!(
        msg.contains("chunk read error"),
        "error must name the interruption kind: {msg:?}"
    );
    assert!(
        msg.contains("after 3 attempts"),
        "error must report attempts: {msg:?}"
    );
}
