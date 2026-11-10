//! Minimal hand-rolled DAG-claim stub server for the runner DAG-arm tests.
//!
//! Same wire-contract style as the parent node-API stub (`support/mod.rs`):
//! register + heartbeat (with `cancel_run_ids`) + an always-empty task claim
//! so the runner's idle loop falls through to the DAG claim arm, plus
//! `/api/nodes/dag/claim` and the per-run DAG events/status uploads.
//! Deliberately does NOT import `opencoder-web`: the tests prove the worker
//! side against THE WIRE CONTRACT rather than our own server internals.
//! Every route requires the bearer token via the parent module's shared
//! `auth` middleware, mirroring the real server.

use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use opencoder_core::node_protocol::ClaimResponse;
use opencoder_dag::protocol::{DagClaimedRun, DagEventBatch, DagStatusReport};
use serde_json::json;

/// Bearer credential for these routes: the router reuses the parent stub's
/// `auth` middleware, so it validates the parent stub's shared token.
pub const TOKEN: &str = super::TOKEN;
/// Stable fake node id handed out by every registration.
pub const STUB_NODE_ID: &str = "node-dag-stub-1";

#[derive(Default)]
pub struct Inner {
    pub registered: Vec<String>,
    pub heartbeats: usize,
    /// Handed out on the first claim poll.
    pub queued_run: Option<DagClaimedRun>,
    pub dag_claims: usize,
    pub claimed_run_ids: Vec<String>,
    /// Piggybacked in `cancel_run_ids` when set.
    pub cancel_run_ids: Vec<String>,
    pub dag_statuses: Vec<DagStatusReport>,
}

pub struct Stub {
    pub inner: Mutex<Inner>,
}

impl Stub {
    pub fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("stub state lock")
    }

    /// Queue one run to hand out on the next DAG claim poll.
    pub fn queue_run(&self, run: DagClaimedRun) {
        self.lock().queued_run = Some(run);
    }

    /// Instruct cancellation of `run_id` via the next heartbeat reply.
    pub fn set_cancel_run(&self, run_id: &str) {
        self.lock().cancel_run_ids = vec![run_id.to_string()];
    }

    pub fn heartbeat_count(&self) -> usize {
        self.lock().heartbeats
    }

    pub fn dag_claims(&self) -> usize {
        self.lock().dag_claims
    }
}

// ---------------------------------------------------------------- routes --

async fn register(State(st): State<Arc<Stub>>, Json(body): Json<serde_json::Value>) -> Response {
    st.lock().registered.push(
        body.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    );
    Json(json!({ "node_id": STUB_NODE_ID })).into_response()
}

async fn heartbeat(State(st): State<Arc<Stub>>, Path(_id): Path<String>) -> Response {
    let cancels = {
        let mut g = st.lock();
        g.heartbeats += 1;
        g.cancel_run_ids.clone()
    };
    Json(json!({
        "server_time_ms": 0,
        "cancel_task_ids": [],
        "cancel_run_ids": cancels,
        "controls": [],
    }))
    .into_response()
}

/// No prompt task is ever due: the task claim stays empty so the runner's
/// idle loop falls through to the DAG claim arm.
async fn task_claim() -> Json<ClaimResponse> {
    Json(ClaimResponse::default())
}

async fn dag_claim(
    State(st): State<Arc<Stub>>,
    Query(p): Query<std::collections::HashMap<String, String>>,
) -> Response {
    assert_eq!(p.get("node_id").map(String::as_str), Some(STUB_NODE_ID));
    let mut g = st.lock();
    g.dag_claims += 1;
    match g.queued_run.take() {
        Some(run) => {
            g.claimed_run_ids.push(run.run_id.clone());
            Json(run).into_response()
        }
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn dag_events(Json(batch): Json<DagEventBatch>) -> StatusCode {
    assert!(!batch.events.is_empty());
    StatusCode::OK
}

async fn dag_status(
    State(st): State<Arc<Stub>>,
    Path(_rid): Path<String>,
    Json(report): Json<DagStatusReport>,
) -> StatusCode {
    st.lock().dag_statuses.push(report);
    StatusCode::OK
}

fn router(st: Arc<Stub>) -> Router {
    Router::new()
        .route("/api/nodes/register", post(register))
        .route("/api/nodes/:id/heartbeat", post(heartbeat))
        .route("/api/nodes/tasks/claim", get(task_claim))
        .route("/api/nodes/dag/claim", get(dag_claim))
        .route("/api/nodes/dag/runs/:rid/events", post(dag_events))
        .route("/api/nodes/dag/runs/:rid/status", post(dag_status))
        .layer(middleware::from_fn(super::auth))
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
