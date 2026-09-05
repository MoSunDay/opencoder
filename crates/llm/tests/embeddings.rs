//! Integration tests for `ChatStream::embed` against a local fake
//! `/embeddings` server (plain TCP, no extra dependencies, no real network:
//! everything stays on 127.0.0.1).
//!
//! Covers the full client path — URL joining (`{base}/embeddings`), auth +
//! JSON headers, request-body shape, response parsing (including out-of-order
//! `index` re-sorting), and upstream-error context — through the sync `embed`
//! trait method from three caller contexts: a multi-thread runtime, a
//! current-thread runtime, and no runtime at all.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use opencoder_llm::{build_embed_body, ChatClient, ChatStream};

/// A single canned response for the fake server.
struct Resp {
    status: &'static str,
    json: String,
    /// Extra raw header lines (e.g. `retry-after: 0`) appended to the reply,
    /// so tests can exercise header-driven client behavior.
    extra_headers: Vec<String>,
}

fn ok(json: String) -> Resp {
    Resp {
        status: "200 OK",
        json,
        extra_headers: Vec::new(),
    }
}

fn err_resp(status: &'static str, json: &str) -> Resp {
    Resp {
        status,
        json: json.to_string(),
        extra_headers: Vec::new(),
    }
}

/// Attach one extra header to a canned response (pure: consumes and returns
/// the response value).
fn with_header(resp: Resp, name: &str, value: &str) -> Resp {
    let Resp {
        status,
        json,
        mut extra_headers,
    } = resp;
    extra_headers.push(format!("{name}: {value}"));
    Resp {
        status,
        json,
        extra_headers,
    }
}

/// Capture channel shared with the fake server: one raw request (start line +
/// headers + body) per served connection, so `len()` is the request count.
type Captured = Arc<Mutex<Vec<String>>>;

/// Read a single HTTP request (headers, then exactly `content-length` bytes
/// of body) from `stream` and return it as one lossy string. Byte-at-a-time
/// reads keep this dependency-free; embeddings requests are tiny.
fn read_request(stream: &mut TcpStream) -> Option<String> {
    // Read headers first...
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte).unwrap_or(0);
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&byte);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let headers = String::from_utf8_lossy(&buf).to_string();
    // ...then exactly content-length bytes of body.
    let len: usize = headers
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())?
        })
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).expect("read request body");
    Some(format!("{headers}{}", String::from_utf8_lossy(&body)))
}

/// Reply with a canned response and close the connection (`connection:
/// close`), so every client attempt lands on a fresh connection and consumes
/// exactly one entry of the response sequence.
fn write_reply(stream: &mut TcpStream, resp: &Resp) {
    let payload = &resp.json;
    let extra = resp
        .extra_headers
        .iter()
        .map(|h| format!("{h}\r\n"))
        .collect::<String>();
    let reply = format!(
        "HTTP/1.1 {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n{}\r\n{}",
        resp.status,
        payload.len(),
        extra,
        payload
    );
    stream.write_all(reply.as_bytes()).unwrap();
    stream.flush().unwrap();
}

/// Serve the `resps` sequence in order — one canned response per accepted
/// connection, then the listener closes (any further attempt gets a refused
/// connection) — recording every raw request into `captured`. Runs on a
/// plain std thread so it works under every caller context.
fn serve_many(listener: TcpListener, captured: Captured, resps: Vec<Resp>) {
    std::thread::spawn(move || {
        for resp in resps {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let raw = match read_request(&mut stream) {
                Some(raw) => raw,
                None => return,
            };
            captured.lock().unwrap().push(raw);
            write_reply(&mut stream, &resp);
        }
    });
}

/// Single-response convenience wrapper over [`serve_many`].
fn serve_one(listener: TcpListener, captured: Captured, resp: Resp) {
    serve_many(listener, captured, vec![resp]);
}

/// Bind the fake server, returning `(listener, base_url)`.
fn bind_server() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    (listener, format!("http://{addr}"))
}

/// Bind the fake server for a single response, return `(base_url,
/// captured-request channel)`.
fn start(resp: Resp) -> (String, Captured) {
    let (listener, base_url) = bind_server();
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    serve_one(listener, captured.clone(), resp);
    (base_url, captured)
}

/// Bind the fake server for an ordered sequence of responses.
fn start_many(resps: Vec<Resp>) -> (String, Captured) {
    let (listener, base_url) = bind_server();
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    serve_many(listener, captured.clone(), resps);
    (base_url, captured)
}

fn client(base_url: &str) -> ChatClient {
    ChatClient::new(base_url, "test-key", &[], None).expect("build client")
}

fn embeddings_payload() -> String {
    // Deliberately shuffled: index 1 arrives before index 0.
    r#"{"object":"list","data":[
        {"object":"embedding","index":1,"embedding":[0.0,0.6,0.8]},
        {"object":"embedding","index":0,"embedding":[0.3,0.4,0.5]}
    ],"model":"text-embedding-3-small","usage":{"prompt_tokens":2,"total_tokens":2}}"#
        .to_string()
}

fn one_entry_payload() -> String {
    r#"{"object":"list","data":[
        {"object":"embedding","index":0,"embedding":[0.3,0.4,0.5]}
    ],"model":"m","usage":{"prompt_tokens":1,"total_tokens":1}}"#
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embed_posts_and_reorders_via_index() {
    let (base_url, captured) = start(ok(embeddings_payload()));
    let texts = vec!["first".to_string(), "second".to_string()];
    let vecs = ChatStream::embed(&client(&base_url), &texts, "text-embedding-3-small")
        .expect("embed succeeds");

    assert_eq!(vecs.len(), 2);
    assert_eq!(vecs[0], vec![0.3, 0.4, 0.5], "index 0 maps to input 0");
    assert_eq!(vecs[1], vec![0.0, 0.6, 0.8], "index 1 maps to input 1");

    let raw = captured
        .lock()
        .unwrap()
        .last()
        .cloned()
        .expect("request captured");
    assert!(raw.starts_with("POST /embeddings HTTP/1.1"), "url: {raw}");
    assert!(raw.contains("authorization: Bearer test-key"), "{raw}");
    assert!(raw.contains("content-type: application/json"), "{raw}");
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or("");
    assert_eq!(
        body,
        build_embed_body(&texts, "text-embedding-3-small"),
        "request body matches the pure builder"
    );
}

#[tokio::test]
async fn embed_works_on_a_current_thread_runtime() {
    // `#[tokio::test]` defaults to the current-thread flavor; `embed` must
    // still complete (it drives the POST on a helper thread's own runtime).
    let (base_url, _captured) = start(ok(one_entry_payload()));
    let vecs = ChatStream::embed(&client(&base_url), &["x".to_string()], "m")
        .expect("embed on current-thread runtime");
    assert_eq!(vecs, vec![vec![0.3, 0.4, 0.5]]);
}

#[test]
fn embed_works_without_any_runtime() {
    let (base_url, _captured) = start(ok(one_entry_payload()));
    let vecs = ChatStream::embed(&client(&base_url), &["x".to_string()], "m")
        .expect("embed from a sync caller");
    assert_eq!(vecs, vec![vec![0.3, 0.4, 0.5]]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embed_surfaces_upstream_error_with_status_and_body() {
    // 500 is retryable, so the same canned error must be served on every
    // attempt of the bounded budget before the final failure surfaces it.
    let err500 = || {
        err_resp(
            "500 Internal Server Error",
            r#"{"error":{"message":"boom"}}"#,
        )
    };
    let (base_url, _captured) = start_many(vec![err500(), err500(), err500()]);
    let err = ChatStream::embed(&client(&base_url), &["x".to_string()], "m")
        .expect_err("upstream 500 must fail");
    let msg = format!("{err:#}");
    assert!(msg.contains("500"), "status missing: {msg}");
    assert!(msg.contains("boom"), "body missing: {msg}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embed_keeps_arrival_order_without_index_field() {
    let payload = r#"{"data":[{"embedding":[9.0]},{"embedding":[8.0]}]}"#.to_string();
    let (base_url, _captured) = start(ok(payload));
    let vecs = ChatStream::embed(&client(&base_url), &["a".to_string(), "b".to_string()], "m")
        .expect("embed without index fields");
    assert_eq!(vecs, vec![vec![9.0], vec![8.0]]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embed_rejects_vector_count_mismatch() {
    // Server returns two vectors for one input text — misaligned data must
    // surface as an error, never silently pass through.
    let (base_url, _captured) = start(ok(embeddings_payload()));
    let err = ChatStream::embed(&client(&base_url), &["only-one".to_string()], "m")
        .expect_err("count mismatch must fail");
    let msg = format!("{err:#}");
    assert!(msg.contains("2 vectors for 1 input texts"), "got: {msg}");
}

// ---- bounded retry behavior (transient upstream blips) ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embed_retries_transient_500_then_succeeds() {
    // The first attempt trips a retryable 500; the retry must re-send the
    // same request and succeed. Exactly one retry (2 requests total), which
    // also proves the body/headers are rebuilt per attempt.
    let (base_url, captured) = start_many(vec![
        err_resp(
            "500 Internal Server Error",
            r#"{"error":{"message":"blip"}}"#,
        ),
        ok(one_entry_payload()),
    ]);
    let vecs = ChatStream::embed(&client(&base_url), &["x".to_string()], "m")
        .expect("retry recovers from a transient 500");
    assert_eq!(vecs, vec![vec![0.3, 0.4, 0.5]]);
    assert_eq!(captured.lock().unwrap().len(), 2, "exactly one retry");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embed_honors_retry_after_header_on_429() {
    // 429 + `Retry-After: 0`: the hint is floored to the 1 s minimum by
    // `retry_delay`, so the single pre-retry sleep must last at least 1 s —
    // strictly longer than the bare 0.5–0.75 s jittered backoff, which is
    // the observable proof that the header was honored. Lower bound only:
    // an upper bound would flake on slow machines. (The large-hint cap is
    // already unit-tested in `retry.rs`; no test here sleeps near it.)
    let (base_url, captured) = start_many(vec![
        with_header(
            err_resp(
                "429 Too Many Requests",
                r#"{"error":{"message":"slow down"}}"#,
            ),
            "retry-after",
            "0",
        ),
        ok(one_entry_payload()),
    ]);
    let started = std::time::Instant::now();
    let vecs = ChatStream::embed(&client(&base_url), &["x".to_string()], "m")
        .expect("retry after Retry-After hint recovers");
    assert_eq!(vecs, vec![vec![0.3, 0.4, 0.5]]);
    assert!(
        started.elapsed() >= std::time::Duration::from_secs(1),
        "Retry-After: 0 must floor to >=1s, elapsed {:?}",
        started.elapsed()
    );
    assert_eq!(captured.lock().unwrap().len(), 2, "exactly one retry");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embed_fails_fast_on_non_retryable_400() {
    // 400 is off the retry whitelist: the error must surface immediately with
    // status and body, and no second request may leave the client.
    let (base_url, captured) = start_many(vec![err_resp(
        "400 Bad Request",
        r#"{"error":{"message":"bad input"}}"#,
    )]);
    let err = ChatStream::embed(&client(&base_url), &["x".to_string()], "m")
        .expect_err("400 must fail immediately");
    let msg = format!("{err:#}");
    assert!(msg.contains("400"), "status missing: {msg}");
    assert!(msg.contains("bad input"), "body missing: {msg}");
    assert_eq!(captured.lock().unwrap().len(), 1, "no retry for 4xx");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embed_exhausts_retry_budget_on_persistent_500() {
    // Three 500s exhaust the 1-initial + 2-retry budget: the third failure is
    // terminal, the message carries the attempts context, and a fourth
    // request is never issued.
    let err500 = || {
        err_resp(
            "500 Internal Server Error",
            r#"{"error":{"message":"down"}}"#,
        )
    };
    let (base_url, captured) = start_many(vec![err500(), err500(), err500()]);
    let err = ChatStream::embed(&client(&base_url), &["x".to_string()], "m")
        .expect_err("persistent 500 must fail");
    let msg = format!("{err:#}");
    assert!(msg.contains("500"), "status missing: {msg}");
    assert!(msg.contains("after 3 attempts"), "attempts missing: {msg}");
    assert_eq!(
        captured.lock().unwrap().len(),
        3,
        "budget is exactly 3 attempts"
    );
}
