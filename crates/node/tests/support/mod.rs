//! Minimal hand-rolled node-API stub server shared by the runner-loop tests.
//!
//! Deliberately does NOT import `opencoder-web`: it re-implements only the
//! five REST endpoints the protocol needs (`register`, `heartbeat`, `claim`,
//! `events` upload, `status`) over plain in-memory state, so the tests prove
//! the worker side against THE WIRE CONTRACT rather than against our own
//! server internals. Every route requires the bearer token, mirroring auth.

#![allow(dead_code)] // helpers are shared per-test-file; not all are used by both

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use opencoder_core::node_protocol::{
    ClaimResponse, ClaimedTask, ControlTask, FetchMessagesResult, NodeEventBatch,
    NodeHeartbeatResponse, NodeStatusReport,
};

pub const TOKEN: &str = "node-stub-bearer";
/// Stable fake node id handed out by every registration.
pub const STUB_NODE_ID: &str = "node-stub-1";

#[derive(Debug, Clone)]
pub struct QueuedTask {
    pub task_id: String,
    pub session_id: String,
    pub prompt: String,
}

#[derive(Default)]
pub struct Inner {
    pub registered: Vec<String>,
    pub heartbeats: usize,
    /// Tasks waiting to be claimed (oldest first).
    pub queue: VecDeque<QueuedTask>,
    /// Task ids this stub has handed out via claim.
    pub claimed: Vec<String>,
    /// Pending cancel instructions (consumed on next heartbeat).
    pub cancels: Vec<String>,
    /// Uploaded events in strict arrival order.
    pub events: Vec<opencoder_core::node_protocol::NodeEventIn>,
    pub batches: usize,
    /// Terminal reports in arrival order: (task_id, status, error).
    pub statuses: Vec<(String, String, Option<String>)>,
    /// Control tasks waiting for delivery (claim reply or heartbeat batch).
    pub controls: VecDeque<ControlTask>,
    /// Control results uploaded by the worker (arrival order).
    pub control_results: Vec<FetchMessagesResult>,
    /// When set, every heartbeat request parks until this instant BEFORE
    /// touching shared state — the "wedged server" simulation for heartbeat
    /// budget tests. A parked beat neither counts nor drains cancels/controls
    /// early; once the instant passes (or it was never set) the beat is
    /// served normally.
    pub hang_heartbeats_until: Option<Instant>,
}

pub struct Stub {
    pub inner: Mutex<Inner>,
}

impl Stub {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("stub state lock")
    }

    /// Enqueue one dispatchable task before the runner polls.
    pub fn push_task(&self, prompt: &str) -> QueuedTask {
        let t = QueuedTask {
            task_id: format!("task-{}", ulid_like()),
            session_id: format!("sess-{}", ulid_like()),
            prompt: prompt.to_string(),
        };
        self.lock().queue.push_back(t.clone());
        t
    }

    pub fn request_cancel(&self, tid: &str) {
        self.lock().cancels.push(tid.to_string());
    }

    pub fn claimed(&self) -> Vec<String> {
        self.lock().claimed.clone()
    }

    pub fn statuses(&self) -> Vec<(String, String, Option<String>)> {
        self.lock().statuses.clone()
    }

    pub fn status_of(&self, tid: &str) -> Option<String> {
        self.statuses()
            .into_iter()
            .find(|(id, _, _)| id == tid)
            .map(|(_, s, _)| s)
    }

    /// Snapshot of uploaded events (arrival order preserved).
    pub fn events(&self) -> Vec<opencoder_core::node_protocol::NodeEventIn> {
        self.lock().events.clone()
    }

    /// Number of distinct upload batches received so far.
    pub fn batch_count(&self) -> usize {
        self.lock().batches
    }

    pub fn heartbeat_count(&self) -> usize {
        self.lock().heartbeats
    }

    pub fn registrations(&self) -> Vec<String> {
        self.lock().registered.clone()
    }

    /// Queue one control task for delivery on the next claim/heartbeat.
    pub fn push_control(&self, task: ControlTask) {
        self.lock().controls.push_back(task);
    }

    /// Snapshot of uploaded control results.
    pub fn control_results(&self) -> Vec<FetchMessagesResult> {
        self.lock().control_results.clone()
    }

    /// Make every heartbeat request park for `d` before answering. Callers
    /// with a heartbeat budget shorter than `d` observe a client-side
    /// timeout; beats arriving after the window lapses are served normally,
    /// which is what makes the recovery assertions deterministic.
    pub fn hang_heartbeats_for(&self, d: Duration) {
        self.lock().hang_heartbeats_until = Some(Instant::now() + d);
    }
}

/// Tiny unique-suffix helper (avoids an extra dev-only dependency).
fn ulid_like() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    format!("{:x}{:x}", now.as_nanos(), n)
}

// ---------------------------------------------------------------- routes --

async fn register(State(st): State<Arc<Stub>>, Json(body): Json<serde_json::Value>) -> Response {
    st.lock().registered.push(
        body.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    );
    Json(serde_json::json!({ "node_id": STUB_NODE_ID })).into_response()
}

async fn heartbeat(State(st): State<Arc<Stub>>, Path(_id): Path<String>) -> Response {
    // Simulated wedge: park BEFORE taking the state lock so a hung beat
    // neither counts nor consumes cancels/controls ahead of its time. The
    // client usually times out meanwhile; the late response is discarded by
    // the transport, which is exactly the slow-server behavior under test.
    let hang_until = st.lock().hang_heartbeats_until;
    if let Some(until) = hang_until {
        let now = Instant::now();
        if now < until {
            tokio::time::sleep(until - now).await;
        }
    }
    // Heartbeat counting shares the lock with cancel consumption so a test can
    // never race its own cancellation window.
    let mut g = st.lock();
    g.heartbeats += 1;
    let ids = std::mem::take(&mut g.cancels);
    // Server contract: a busy node receives up to 4 queued control tasks.
    let mut controls = Vec::new();
    while controls.len() < 4 {
        match g.controls.pop_front() {
            Some(t) => controls.push(t),
            None => break,
        }
    }
    drop(g);
    Json(NodeHeartbeatResponse {
        server_time_ms: 0,
        cancel_task_ids: ids,
        cancel_run_ids: vec![],
        controls,
    })
    .into_response()
}

#[derive(serde::Deserialize)]
pub struct ClaimQuery {
    pub node_id: String,
}

async fn claim(State(st): State<Arc<Stub>>, Query(q): Query<ClaimQuery>) -> Response {
    let mut g = st.lock();
    // Server contract: durable task preferred; control rides along only when
    // no durable task was due. 204 when both absent.
    if let Some(t) = g.queue.pop_front() {
        g.claimed.push(t.task_id.clone());
        drop(g);
        return Json(ClaimResponse {
            task: Some(ClaimedTask {
                task_id: t.task_id,
                session_id: t.session_id,
                title: None,
                prompt: t.prompt,
                agent: None,
                model: None,
                created_at: 0,
            }),
            control: None,
        })
        .into_response();
    }
    let control = g.controls.pop_front();
    drop(g);
    match control {
        Some(c) => Json(ClaimResponse {
            task: None,
            control: Some(c),
        })
        .into_response(),
        None => {
            let _ = q.node_id;
            StatusCode::NO_CONTENT.into_response()
        }
    }
}

async fn upload_events(
    State(st): State<Arc<Stub>>,
    Path(tid): Path<String>,
    Json(batch): Json<NodeEventBatch>,
) -> Response {
    let mut g = st.lock();
    g.batches += 1;
    for ev in batch.events {
        g.events.push(ev);
    }
    let _ = tid;
    Json(serde_json::json!({ "appended": 1 })).into_response()
}

async fn report_status(
    State(st): State<Arc<Stub>>,
    Path(tid): Path<String>,
    Json(report): Json<NodeStatusReport>,
) -> Response {
    if !report.validate() {
        return (StatusCode::BAD_REQUEST, "invalid status").into_response();
    }
    st.lock().statuses.push((tid, report.status, report.error));
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn control_result(
    State(st): State<Arc<Stub>>,
    Path(_id): Path<String>,
    Json(result): Json<FetchMessagesResult>,
) -> Response {
    st.lock().control_results.push(result);
    Json(serde_json::json!({ "resolved": true })).into_response()
}

/// HMAC-signature gate mirroring the server's `auth_sig_mw`: every request
/// must carry a valid `x-sig` / `x-sig-timestamp` pair over
/// `{METHOD}\n{path_and_query}\n{ts}\n{sha256(body)}` with the shared token.
/// The body is buffered, verified, and re-injected for the `Json` extractors.
async fn auth(req: Request<axum::body::Body>, next: Next) -> Response {
    use axum::body::to_bytes;
    let (parts, body) = req.into_parts();
    let bytes = match to_bytes(body, 8 << 20).await {
        Ok(b) => b.to_vec(),
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let ts_raw = parts
        .headers
        .get(opencoder_core::auth_sig::TS_HEADER)
        .and_then(|v| v.to_str().ok());
    let sig_raw = parts
        .headers
        .get(opencoder_core::auth_sig::SIG_HEADER)
        .and_then(|v| v.to_str().ok());
    let (Some(ts_raw), Some(sig_raw)) = (ts_raw, sig_raw) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(ts_ms) = ts_raw.trim().parse::<i64>() else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let pq = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let ok = opencoder_core::auth_sig::verify(
        TOKEN,
        parts.method.as_str(),
        &pq,
        ts_ms,
        now_ms,
        &bytes,
        sig_raw,
    )
    .is_ok();
    if !ok {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let req = Request::from_parts(parts, axum::body::Body::from(bytes));
    next.run(req).await
}

fn router(st: Arc<Stub>) -> Router {
    Router::new()
        .route("/api/nodes/register", post(register))
        .route("/api/nodes/:id/heartbeat", post(heartbeat))
        .route("/api/nodes/tasks/claim", get(claim))
        .route("/api/nodes/tasks/:tid/events", post(upload_events))
        .route("/api/nodes/tasks/:tid/status", post(report_status))
        .route("/api/nodes/:id/control_result", post(control_result))
        .layer(middleware::from_fn(auth))
        .with_state(st)
}

/// Spawn the stub on an ephemeral loopback port; returns (base_url, stub).
/// The returned handle is used for assertions + pre-claim seeding.
pub async fn spawn_stub() -> (String, Arc<Stub>) {
    let st = Arc::new(Stub {
        inner: Mutex::new(Inner::default()),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = st.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router(served)).await;
    });
    (format!("http://{addr}"), st)
}

/// Poll `probe` until it yields `Some` or the deadline expires (20 ms cadence).
pub async fn wait_for<T>(secs: u64, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = probe() {
            return v;
        }
        assert!(
            Instant::now() < deadline,
            "condition did not settle within {secs}s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
