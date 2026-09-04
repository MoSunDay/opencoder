//! Runner-loop tests for the DAG claim arm: with a `DagHook` injected, an
//! idle node (no prompt tasks due) polls `/api/nodes/dag/claim`, executes
//! the claimed run through the hook under its own heartbeater, and a
//! heartbeat `cancel_run_ids` flip converges into the hook's cancel flag.
//! Self-contained mini-stub (`support/` wire-contract style); the fake
//! hook's `claim` goes through the REAL signed uplink (200/204 exercised).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::{Path, Query, State};
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use opencoder_core::node_protocol::ClaimResponse;
use opencoder_dag::protocol::{DagClaimedRun, DagEventBatch, DagStatusReport};
use opencoder_dag::{DagSpec, StepKind, StepSpec};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_node::uplink::Uplink;
use opencoder_node::{DagHook, NodeOpts};
use serde_json::json;
use tokio::sync::{watch, Notify};

const TOKEN: &str = "dag-stub-bearer";
const STUB_NODE_ID: &str = "node-dag-stub-1";

#[derive(Default)]
struct Inner {
    registered: Vec<String>,
    heartbeats: usize,
    /// Handed out on the first claim poll.
    queued_run: Option<DagClaimedRun>,
    dag_claims: usize,
    claimed_run_ids: Vec<String>,
    /// Piggybacked in `cancel_run_ids` when set.
    cancel_run_ids: Vec<String>,
    dag_statuses: Vec<DagStatusReport>,
}

struct Stub {
    inner: Mutex<Inner>,
}

impl Stub {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("stub state lock")
    }

    fn queue_run(&self, run: DagClaimedRun) {
        self.lock().queued_run = Some(run);
    }

    fn set_cancel_run(&self, run_id: &str) {
        self.lock().cancel_run_ids = vec![run_id.to_string()];
    }

    fn heartbeat_count(&self) -> usize {
        self.lock().heartbeats
    }
    fn dag_claims(&self) -> usize {
        self.lock().dag_claims
    }
}

fn header_str(headers: &axum::http::HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

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

/// Same bearer+signature contract as the real server (mirrors `support/`).
async fn auth(mut req: Request<axum::body::Body>, next: Next) -> Response {
    use opencoder_core::auth_sig;
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 2 * 1024 * 1024).await.unwrap();
    let ts = header_str(&parts.headers, "x-sig-timestamp")
        .parse::<i64>()
        .unwrap_or_default();
    let sig_raw = header_str(&parts.headers, "x-sig");
    let pq = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let ok = auth_sig::verify(
        TOKEN,
        parts.method.as_str(),
        &pq,
        ts,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64,
        &bytes,
        &sig_raw,
    )
    .is_ok();
    if !ok {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    req = Request::from_parts(parts, axum::body::Body::from(bytes));
    next.run(req).await
}

async fn spawn_stub() -> (String, Arc<Stub>) {
    let st = Arc::new(Stub {
        inner: Mutex::new(Inner::default()),
    });
    let app = Router::new()
        .route("/api/nodes/register", post(register))
        .route("/api/nodes/:id/heartbeat", post(heartbeat))
        .route("/api/nodes/tasks/claim", get(task_claim))
        .route("/api/nodes/dag/claim", get(dag_claim))
        .route("/api/nodes/dag/runs/:rid/events", post(dag_events))
        .route("/api/nodes/dag/runs/:rid/status", post(dag_status))
        .layer(middleware::from_fn(auth))
        .with_state(Arc::clone(&st));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), st)
}

async fn wait_for<T>(secs: u64, mut probe: impl FnMut() -> Option<T>) -> T {
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

/// Fake executor: `claim` uses the REAL signed uplink; `execute` records the
/// run and (optionally) parks until the runner flips the cancel flag.
struct FakeHook {
    uplink: Uplink,
    claim_node_ids: Mutex<Vec<String>>,
    executed: Mutex<Vec<String>>,
    started: Arc<Notify>,
    finished: Arc<Notify>,
    park_for_cancel: bool,
    observed_cancel: AtomicBool,
}

#[async_trait]
impl DagHook for FakeHook {
    async fn claim(&self, node_id: &str) -> anyhow::Result<Option<DagClaimedRun>> {
        self.claim_node_ids
            .lock()
            .unwrap()
            .push(node_id.to_string());
        self.uplink.dag_claim(node_id).await
    }

    async fn execute(
        &self,
        run: DagClaimedRun,
        mut cancel_rx: watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        self.executed.lock().unwrap().push(run.run_id.clone());
        self.started.notify_one();
        if self.park_for_cancel {
            while !*cancel_rx.borrow_and_update() {
                if cancel_rx.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
            self.observed_cancel.store(true, Ordering::SeqCst);
        }
        self.finished.notify_one();
        Ok(())
    }
}

fn queued_run(run_id: &str) -> DagClaimedRun {
    DagClaimedRun {
        run_id: run_id.to_string(),
        dag_id: "dag-1".to_string(),
        spec: DagSpec {
            name: "stub-flow".into(),
            description: None,
            steps: vec![StepSpec {
                name: "only".into(),
                depends_on: vec![],
                kind: StepKind::Agent {
                    prompt: "p".into(),
                    agent: None,
                    model: None,
                },
                timeout_secs: None,
            }],
        },
        created_at: 0,
    }
}

fn test_opts(
    base: &str,
    workdir: &std::path::Path,
    data: &std::path::Path,
    dag: Arc<dyn DagHook>,
) -> NodeOpts {
    NodeOpts {
        name: "node-dag".into(),
        remote: base.into(),
        token: TOKEN.into(),
        workdir: workdir.to_path_buf(),
        heartbeat_interval: Duration::from_millis(40),
        claim_interval: Duration::from_millis(30),
        version: env!("CARGO_PKG_VERSION").into(),
        local_store_dir: Some(data.to_path_buf()),
        dag: Some(dag),
    }
}

fn mock_client() -> Arc<dyn ChatStream> {
    Arc::new(MockChatClient::new())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_loop_claims_dag_run_and_executes_it_via_hook() {
    let (base, st) = spawn_stub().await;
    let workdir = tempfile::tempdir().unwrap().keep();
    let data = tempfile::tempdir().unwrap();
    st.queue_run(queued_run("run-happy"));

    let hook = Arc::new(FakeHook {
        uplink: Uplink::new(&base, TOKEN).unwrap(),
        claim_node_ids: Mutex::new(vec![]),
        executed: Mutex::new(vec![]),
        started: Arc::new(Notify::new()),
        finished: Arc::new(Notify::new()),
        park_for_cancel: false,
        observed_cancel: AtomicBool::new(false),
    });

    let runner = tokio::spawn(opencoder_node::run_node(
        test_opts(
            &base,
            &workdir,
            data.path(),
            hook.clone() as Arc<dyn DagHook>,
        ),
        Some(mock_client()),
    ));

    // Registration precedes any work; the run is claimed exactly once.
    wait_for(30, || (!st.lock().registered.is_empty()).then_some(())).await;
    let ids = wait_for(30, || {
        let ids = st.lock().claimed_run_ids.clone();
        (!ids.is_empty()).then_some(ids)
    })
    .await;
    assert_eq!(ids, vec!["run-happy".to_string()]);

    // The hook executed it, claiming under the registered node id.
    wait_for(30, || {
        let ex = hook.executed.lock().unwrap().clone();
        (!ex.is_empty()).then_some(ex)
    })
    .await;
    assert_eq!(hook.executed.lock().unwrap().as_slice(), ["run-happy"]);
    assert!(hook
        .claim_node_ids
        .lock()
        .unwrap()
        .iter()
        .all(|id| id == STUB_NODE_ID));
    assert!(!hook.observed_cancel.load(Ordering::SeqCst));

    // The per-run heartbeater kept the node live during execution.
    wait_for(30, || (st.heartbeat_count() >= 1).then_some(())).await;
    // The fake executor uploads nothing itself — no stray dag traffic.
    assert!(st.lock().dag_statuses.is_empty());
    // Later claim polls see 204 and the loop stays idle.
    wait_for(30, || (st.dag_claims() >= 2).then_some(())).await;

    runner.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heartbeat_cancel_run_ids_flips_the_hook_cancel_flag() {
    let (base, st) = spawn_stub().await;
    let workdir = tempfile::tempdir().unwrap().keep();
    let data = tempfile::tempdir().unwrap();
    st.queue_run(queued_run("run-stop"));

    let hook = Arc::new(FakeHook {
        uplink: Uplink::new(&base, TOKEN).unwrap(),
        claim_node_ids: Mutex::new(vec![]),
        executed: Mutex::new(vec![]),
        started: Arc::new(Notify::new()),
        finished: Arc::new(Notify::new()),
        park_for_cancel: true,
        observed_cancel: AtomicBool::new(false),
    });

    let runner = tokio::spawn(opencoder_node::run_node(
        test_opts(
            &base,
            &workdir,
            data.path(),
            hook.clone() as Arc<dyn DagHook>,
        ),
        Some(mock_client()),
    ));

    // Wait until the hook is INSIDE execute(), then instruct cancellation.
    wait_for(30, || {
        (!hook.executed.lock().unwrap().is_empty()).then_some(())
    })
    .await;
    st.set_cancel_run("run-stop");

    // The parked hook only returns once the flag flipped.
    wait_for(30, || {
        hook.observed_cancel.load(Ordering::SeqCst).then_some(())
    })
    .await;
    assert_eq!(
        st.lock().claimed_run_ids.clone(),
        vec!["run-stop".to_string()]
    );

    runner.abort();
}
