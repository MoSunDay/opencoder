//! Shared harness for the Phase-4 process-level e2e tests: a REAL
//! `build_app` server (web=true, HMAC signature token) bound to a random local port,
//! plus the small browser-side utilities (reqwest client, SSE line reader,
//! poll helper) the flow/reconnect scenarios drive.

#![allow(dead_code)] // each test file uses a different subset

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::support::sig_headers;
use futures::StreamExt;
use opencoder_llm::MockChatClient;
use opencoder_store::LibsqlStore;
use serde_json::Value;

pub const TOKEN: &str = "nodes-e2e-token";

pub struct Server {
    pub base: String,
    pub store: Arc<dyn opencoder_store::Store>,
    pub shutdown: tokio::sync::watch::Sender<bool>,
}

/// Spawn the real router on 127.0.0.1:0 and return its base URL + the shared
/// store (the same Arc the AppState owns, for durable-side reconciliation).
pub async fn spawn_server() -> Server {
    let store: Arc<dyn opencoder_store::Store> =
        Arc::new(LibsqlStore::open_memory().await.unwrap());
    let state = Arc::new(opencoder_web::AppState {
        store: Arc::clone(&store),
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        client_override: Some(Arc::new(MockChatClient::new())),
    });
    let app = opencoder_web::build_app(state, Some(TOKEN.to_string()), true);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let graceful = axum::serve(listener, app).with_graceful_shutdown(async move {
            let mut rx = rx;
            while !*rx.borrow_and_update() {
                if rx.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        });
        let _ = graceful.await;
    });
    Server {
        base: format!("http://{addr}"),
        store,
        shutdown: tx,
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

/// Client for long-lived SSE reads: NO total-request timeout. A loaded box
/// can keep a task's stream open for minutes; the default client's 30 s cap
/// would amputate the stream mid-run and masquerade as frame loss.
pub fn http_sse() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

fn url(base: &str, path: &str) -> String {
    format!("{base}{path}")
}

/// Signed GET returning JSON.
pub async fn get_json(base: &str, path: &str) -> (reqwest::StatusCode, Value) {
    let r = signed_raw("GET", base, path, None).send().await.unwrap();
    let status = r.status();
    let v = r.json::<Value>().await.unwrap_or(Value::Null);
    (status, v)
}

/// Signed POST with an optional JSON body.
pub async fn post_json(
    base: &str,
    path: &str,
    body: Option<Value>,
) -> (reqwest::StatusCode, Value) {
    let bytes = body
        .as_ref()
        .map(|j| serde_json::to_vec(j).unwrap())
        .unwrap_or_default();
    let mut b = signed_raw("POST", base, path, Some(bytes));
    if body.is_none() {
        b = b.header("content-type", "application/json");
    }
    let r = b.send().await.unwrap();
    let status = r.status();
    let v = r.json::<Value>().await.unwrap_or(Value::Null);
    (status, v)
}

/// Build a signed reqwest request over the exact serialized body so the
/// signature matches what the server hashes. `path` must include the query.
fn signed_raw(
    method: &str,
    base: &str,
    path: &str,
    body: Option<Vec<u8>>,
) -> reqwest::RequestBuilder {
    let bytes = body.unwrap_or_default();
    let (tsh, ts, sigh, sig) = sig_headers(TOKEN, method, path, &bytes);
    http()
        .request(
            reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
            url(base, path),
        )
        .header(tsh, ts)
        .header(sigh, sig)
        .header("content-type", "application/json")
        .body(bytes)
}

fn signed_raw_sse(method: &str, base: &str, path: &str) -> reqwest::RequestBuilder {
    let bytes: Vec<u8> = Vec::new();
    let (tsh, ts, sigh, sig) = sig_headers(TOKEN, method, path, &bytes);
    http_sse()
        .request(
            reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
            url(base, path),
        )
        .header(tsh, ts)
        .header(sigh, sig)
        .body(bytes)
}

/// One SSE unit of the fleet API as the browser sees it: the event name plus
/// its JSON payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub kind: String,
    pub data: Value,
}

/// Open `GET path` (signed — EventSource cannot set headers, so the SPA uses
/// fetch streaming with the same header pair) and yield parsed frames until
/// the connection ends.
pub async fn open_sse(base: &str, path: &str) -> impl futures::Stream<Item = Frame> {
    let r = signed_raw_sse("GET", base, path)
        .send()
        .await
        .expect("sse connect");
    parse_sse_response(r)
}

/// Split one chunked HTTP response into axum-SSE frames
/// (`event: <name>` / `data: <json>` blocks separated by blank lines).
/// Boxed+Unpin so test loops can just `.next()` it.
pub fn parse_sse_response(
    resp: reqwest::Response,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = Frame> + Send>> {
    let chunks = resp.bytes_stream();
    let s = futures::stream::unfold((chunks, String::new()), |(mut body, mut buf)| async move {
        loop {
            if let Some(pos) = buf.find("\n\n") {
                let block: String = buf.drain(..pos + 2).collect();
                if let Some(frame) = parse_block(&block) {
                    return Some((frame, (body, buf)));
                }
                continue; // keep-alive comment block
            }
            // Budget generous enough to survive a loaded CI box: the machine
            // may be running many builds/tests concurrently, and a silent
            // frame-gap timeout here masquerades as "stream ended" (loss).
            match tokio::time::timeout(Duration::from_secs(60), body.next()).await {
                Ok(Some(Ok(bytes))) => buf.push_str(&String::from_utf8_lossy(bytes.as_ref())),
                _ => return None, // timeout / end / transport error
            }
        }
    });
    Box::pin(s)
}

fn parse_block(block: &str) -> Option<Frame> {
    let mut kind = None;
    let mut data = String::new();
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            kind = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push_str(rest.trim());
        }
    }
    kind.map(|kind| Frame {
        kind,
        data: serde_json::from_str(&data).unwrap_or(Value::Null),
    })
}

/// Node options tuned for tests (fast heartbeat/claim ticks) matching the
/// conventions of `crates/node/tests`.
pub fn node_opts(
    base: &str,
    name: &str,
    workdir: &std::path::Path,
    data: &std::path::Path,
) -> opencoder_node::NodeOpts {
    opencoder_node::NodeOpts {
        name: name.to_string(),
        remote: base.to_string(),
        token: TOKEN.to_string(),
        workdir: workdir.to_path_buf(),
        heartbeat_interval: Duration::from_millis(40),
        claim_interval: Duration::from_millis(30),
        version: env!("CARGO_PKG_VERSION").to_string(),
        local_store_dir: Some(data.to_path_buf()),
    }
}

/// Pin autopilot off via the project domain file so a developer's global
/// `~/.opencoder/ap.json` cannot append a review turn to the scripted mock
/// round (same trick as `crates/node/tests`).
pub fn pin_autopilot_off(workdir: &std::path::Path) {
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();
}

/// One SSE unit of the fleet API as the browser sees it.
pub async fn spawn_node(
    base: &str,
    name: &str,
    workdir: &std::path::Path,
    data: &std::path::Path,
    client: Arc<dyn opencoder_llm::ChatStream>,
) -> tokio::task::JoinHandle<anyhow::Result<()>> {
    let opts = node_opts(base, name, workdir, data);
    tokio::spawn(opencoder_node::run_node(opts, Some(client)))
}

/// Poll `f` every `tick_ms` until it returns Some or the budget expires.
pub async fn wait_for<T, F: Future<Output = Option<T>>>(
    secs: u64,
    tick_ms: u64,
    mut f: impl FnMut() -> F,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(Instant::now() < deadline, "wait_for budget exhausted");
        tokio::time::sleep(Duration::from_millis(tick_ms)).await;
    }
}

/// Collect SSE frames until `pred` says stop; every frame is returned in
/// arrival order (the terminal one included).
pub async fn collect_until<S>(stream: &mut S, pred: impl Fn(&Frame) -> bool) -> Vec<Frame>
where
    S: futures::Stream<Item = Frame> + Unpin,
{
    let mut out = Vec::new();
    while let Some(frame) = stream.next().await {
        let done = pred(&frame);
        out.push(frame);
        if done {
            break;
        }
    }
    out
}
