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
use axum::http::{header, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use opencoder_core::node_protocol::{
    ClaimedTask, NodeEventBatch, NodeHeartbeatResponse, NodeStatusReport,
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
    // Heartbeat counting shares the lock with cancel consumption so a test can
    // never race its own cancellation window.
    let mut g = st.lock();
    g.heartbeats += 1;
    let ids = std::mem::take(&mut g.cancels);
    drop(g);
    Json(NodeHeartbeatResponse {
        server_time_ms: 0,
        cancel_task_ids: ids,
    })
    .into_response()
}

#[derive(serde::Deserialize)]
pub struct ClaimQuery {
    pub node_id: String,
}

async fn claim(State(st): State<Arc<Stub>>, Query(q): Query<ClaimQuery>) -> Response {
    let mut g = st.lock();
    match g.queue.pop_front() {
        Some(t) => {
            g.claimed.push(t.task_id.clone());
            drop(g);
            Json(ClaimedTask {
                task_id: t.task_id,
                session_id: t.session_id,
                title: None,
                prompt: t.prompt,
                agent: None,
                model: None,
                created_at: 0,
            })
            .into_response()
        }
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

/// Bearer-token gate mirroring the server's auth middleware.
async fn auth(req: Request<axum::body::Body>, next: Next) -> Response {
    let expected = format!("Bearer {TOKEN}");
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        == Some(expected.as_str());
    if !ok {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(req).await
}

fn router(st: Arc<Stub>) -> Router {
    Router::new()
        .route("/api/nodes/register", post(register))
        .route("/api/nodes/:id/heartbeat", post(heartbeat))
        .route("/api/nodes/tasks/claim", get(claim))
        .route("/api/nodes/tasks/:tid/events", post(upload_events))
        .route("/api/nodes/tasks/:tid/status", post(report_status))
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
