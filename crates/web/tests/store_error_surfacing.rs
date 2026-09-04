//! P1#7 + P1#8 regression: storage-layer errors must surface as a structured
//! HTTP 500, never masked as a 404 "not found" nor silently swallowed.
//!
//! - P1#7 `post_skill_store_error_returns_500_not_404`: a `Store::get_session`
//!   failure on POST /skill must reply 500 carrying the store error, NOT a 404
//!   that hides the real cause. `post_skill_nonexistent_returns_404` confirms
//!   the legitimate not-found path still 404s.
//! - P1#8 `post_prompt_skill_persist_error_returns_500`: when `post_prompt`
//!   cannot persist the requested skill (`Store::update_session` error), the
//!   500 must carry a "persist skill" message instead of being dropped by a
//!   `let _ = ...` that swallows the failure.

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use opencoder_core::Message;
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{
    Delivery, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
    SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};
use serde_json::{json, Value};
use tower::ServiceExt;

/// Wraps a real store and delegates every method to `inner`, EXCEPT
/// `get_session` / `update_session` (fail on demand via the two flags) and
/// `append_events` (fails only for the one session named in
/// `fail_append_events_for`). Mirrors the `FailingStore` delegation pattern
/// in `bugfix_contracts.rs`.
struct ErrorStore {
    inner: Arc<dyn Store>,
    fail_get_session: AtomicBool,
    fail_update_session: AtomicBool,
    /// When set, `append_events` fails for batches touching this session id.
    fail_append_events_for: Option<String>,
}

#[async_trait]
impl Store for ErrorStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create_session(&self, meta: &SessionMeta) -> anyhow::Result<()> {
        self.inner.create_session(meta).await
    }
    async fn get_session(&self, id: &str) -> anyhow::Result<Option<SessionMeta>> {
        if self.fail_get_session.load(Ordering::SeqCst) {
            return Err(anyhow::anyhow!("simulated store failure"));
        }
        self.inner.get_session(id).await
    }
    async fn list_sessions(&self, f: &SessionFilter) -> anyhow::Result<Vec<SessionListItem>> {
        self.inner.list_sessions(f).await
    }
    async fn update_session(&self, id: &str, p: &SessionPatch) -> anyhow::Result<()> {
        if self.fail_update_session.load(Ordering::SeqCst) {
            return Err(anyhow::anyhow!("simulated store failure"));
        }
        self.inner.update_session(id, p).await
    }
    async fn delete_session(&self, id: &str) -> anyhow::Result<()> {
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, keep: &str) -> anyhow::Result<u64> {
        self.inner.clear_other_sessions(keep).await
    }
    async fn append_message(&self, sid: &str, m: &Message) -> anyhow::Result<i64> {
        self.inner.append_message(sid, m).await
    }
    async fn append_messages(&self, sid: &str, msgs: &[Message]) -> anyhow::Result<Vec<i64>> {
        self.inner.append_messages(sid, msgs).await
    }
    async fn load_messages(&self, sid: &str) -> anyhow::Result<Vec<Message>> {
        self.inner.load_messages(sid).await
    }
    async fn last_message_seq(&self, sid: &str) -> anyhow::Result<i64> {
        self.inner.last_message_seq(sid).await
    }
    async fn admit_input(&self, input: &SessionInput) -> anyhow::Result<i64> {
        self.inner.admit_input(input).await
    }
    async fn pending_inputs(&self, sid: &str, d: Delivery) -> anyhow::Result<Vec<SessionInput>> {
        self.inner.pending_inputs(sid, d).await
    }
    async fn promote_inputs(&self, sid: &str, up_to: i64, d: Delivery) -> anyhow::Result<Vec<i64>> {
        self.inner.promote_inputs(sid, up_to, d).await
    }
    async fn promote_next_queued(&self, sid: &str) -> anyhow::Result<Option<i64>> {
        self.inner.promote_next_queued(sid).await
    }
    async fn claim_next_queue(&self, sid: &str) -> anyhow::Result<Option<(i64, SessionInput)>> {
        self.inner.claim_next_queue(sid).await
    }
    async fn delete_input(&self, input_id: i64) -> anyhow::Result<()> {
        self.inner.delete_input(input_id).await
    }
    async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> anyhow::Result<()> {
        self.inner.swap_input_order(sid, a, b).await
    }
    async fn append_events(&self, events: &[SessionEventRecord]) -> anyhow::Result<Vec<i64>> {
        if let Some(sid) = self.fail_append_events_for.as_deref() {
            if events.iter().any(|e| e.session_id == sid) {
                return Err(anyhow::anyhow!("simulated closure fan-out failure"));
            }
        }
        self.inner.append_events(events).await
    }
    async fn events_after(&self, sid: &str, after: i64) -> anyhow::Result<Vec<SessionEventRecord>> {
        self.inner.events_after(sid, after).await
    }
    async fn last_event_seq(&self, sid: &str) -> anyhow::Result<i64> {
        self.inner.last_event_seq(sid).await
    }
    async fn create_subagent_task(&self, r: &SubagentTaskRecord) -> anyhow::Result<()> {
        self.inner.create_subagent_task(r).await
    }
    async fn complete_subagent_task(&self, id: &str, result: &str, ok: bool) -> anyhow::Result<()> {
        self.inner.complete_subagent_task(id, result, ok).await
    }
    async fn list_subagent_tasks(&self, sid: &str) -> anyhow::Result<Vec<SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(sid).await
    }
    async fn get_subagent_task(&self, id: &str) -> anyhow::Result<Option<SubagentTaskRecord>> {
        self.inner.get_subagent_task(id).await
    }
    async fn cancel_subagent_task(&self, id: &str) -> anyhow::Result<()> {
        self.inner.cancel_subagent_task(id).await
    }
    // ── node half (F7: registry read path drives the lost-node sweep) ──────
    async fn list_nodes(&self) -> anyhow::Result<Vec<opencoder_store::NodeRecord>> {
        self.inner.list_nodes().await
    }
    async fn converge_lost_node_tasks(
        &self,
        now_ms: i64,
        stale_ms: i64,
    ) -> anyhow::Result<Vec<opencoder_store::NodeTaskRecord>> {
        self.inner.converge_lost_node_tasks(now_ms, stale_ms).await
    }
    // DAG lost-sweep runs on the same read path; forward or the trait's
    // default impl would bail("dag store API is not supported") and 500 the
    // listing under this decorator.
    async fn converge_lost_dag_runs(
        &self,
        now_ms: i64,
        stale_ms: i64,
    ) -> anyhow::Result<Vec<opencoder_store::DagRunRecord>> {
        self.inner.converge_lost_dag_runs(now_ms, stale_ms).await
    }
}

/// Build an AppState backed by `store`, with a fresh tempdir workdir and a
/// `MockChatClient` override so `post_prompt` skips API-key resolution.
async fn state_with_store(store: Arc<dyn Store>) -> Arc<opencoder_web::AppState> {
    let workdir = tempfile::tempdir().unwrap().keep();
    Arc::new(opencoder_web::AppState {
        client_override: Some(Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>),
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store,
        workdir,
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
    })
}

/// Seed a session row (agent "act", model "m").
async fn seed(store: &Arc<dyn Store>, id: &str) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some("act".into()),
            model: Some("m".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

/// Decode a handler response into (status code, JSON body).
async fn decode(resp: axum::response::Response) -> (StatusCode, Value) {
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    (status, v)
}

/// P1#7: a `get_session` store error on POST /skill must surface as a
/// structured 500 — NOT a 404 that masks the real failure.
#[tokio::test]
async fn post_skill_store_error_returns_500_not_404() {
    let inner: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    seed(&inner, "s1").await; // row exists, but the wrapper fails to read it.
    let store: Arc<dyn Store> = Arc::new(ErrorStore {
        inner: inner.clone(),
        fail_get_session: AtomicBool::new(true),
        fail_update_session: AtomicBool::new(false),
        fail_append_events_for: None,
    });
    let state = state_with_store(store).await;
    let app = Router::new()
        .route(
            "/api/sessions/:id/skill",
            post(opencoder_web::api_ops::post_skill),
        )
        .with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/s1/skill")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "skill": "review" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let (status, body) = decode(resp).await;
    assert_ne!(status, StatusCode::NOT_FOUND, "store error must not 404");
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let err = body["error"].as_str().unwrap_or("");
    assert!(
        err.contains("get_session"),
        "error must mention get_session, got: {err}"
    );
}

/// Regression guard for P1#7: the legitimate not-found path still 404s.
#[tokio::test]
async fn post_skill_nonexistent_returns_404() {
    let inner: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let store: Arc<dyn Store> = Arc::new(ErrorStore {
        inner: inner.clone(),
        fail_get_session: AtomicBool::new(false),
        fail_update_session: AtomicBool::new(false),
        fail_append_events_for: None,
    });
    let state = state_with_store(store).await;
    let app = Router::new()
        .route(
            "/api/sessions/:id/skill",
            post(opencoder_web::api_ops::post_skill),
        )
        .with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/ghost/skill")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "skill": "review" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let (status, _body) = decode(resp).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// P1#8: when `post_prompt` cannot persist the requested skill
/// (`update_session` error), it must surface a 500 carrying "persist skill"
/// instead of silently swallowing the failure (`let _ = ...`).
#[tokio::test]
async fn post_prompt_skill_persist_error_returns_500() {
    let inner: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    seed(&inner, "s1").await;
    let store: Arc<dyn Store> = Arc::new(ErrorStore {
        inner: inner.clone(),
        fail_get_session: AtomicBool::new(false),
        fail_update_session: AtomicBool::new(true),
        fail_append_events_for: None,
    });
    let state = state_with_store(store).await;
    let app = Router::new()
        .route(
            "/api/sessions/:id/prompt",
            post(opencoder_web::api::post_prompt),
        )
        .with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/s1/prompt")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "prompt": "hi", "skill": "review" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let (status, body) = decode(resp).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let err = body["error"].as_str().unwrap_or("");
    assert!(
        err.contains("persist skill"),
        "error must mention persist skill, got: {err}"
    );
}

// ── F7: lost-node sweep must not fail the listing when a frame fan-out fails ─

/// `GET /api/nodes` sweeps stale nodes' running tasks to `error("node lost")`
/// and fans the terminal frames out to live subscribers. The sweep is already
/// committed by then, so one failing `emit_closure` must degrade to a log
/// line — an early `return 500` would drop every remaining error frame and
/// hang the SSE clients waiting on them. Here exactly the converged task's
/// `append_events` fails (per-session injection in `ErrorStore`); the listing
/// must still be 200 with the node row present, the transition committed, and
/// (only) the closure frame missing.
#[tokio::test]
async fn lost_node_sweep_emit_failure_does_not_500_list() {
    let inner: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());

    // A node heartbeating far beyond STALE_AFTER_MS with a RUNNING task:
    // registered / dispatched / claimed with backdated timestamps, so the
    // registry read's opportunistic sweep converges it with no raw SQL.
    let now = chrono::Utc::now().timestamp_millis();
    let past = now - 10 * opencoder_web::nodes_state::STALE_AFTER_MS;
    let node = inner
        .register_node("lost-node", None, None, None, past)
        .await
        .unwrap();
    let task = inner
        .dispatch_node_task(
            "task-lost",
            "session-lost",
            &node.id,
            None,
            "job one",
            None,
            None,
            past,
        )
        .await
        .unwrap();
    let claimed = inner.claim_next_node_task(&node.id, past).await.unwrap();
    assert_eq!(
        claimed.as_ref().map(|t| t.id.as_str()),
        Some(task.id.as_str()),
        "precondition: task must be running for the sweep to pick it up"
    );

    let store: Arc<dyn Store> = Arc::new(ErrorStore {
        inner: inner.clone(),
        fail_get_session: AtomicBool::new(false),
        fail_update_session: AtomicBool::new(false),
        fail_append_events_for: Some(task.session_id.clone()),
    });
    let state = state_with_store(store).await;
    let app = Router::new()
        .route("/api/nodes", get(opencoder_web::api_nodes::list_nodes))
        .with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/nodes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, body) = decode(resp).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "frame fan-out failure must not fail the listing: {body}"
    );
    assert!(
        body["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == node.id.as_str()),
        "fleet rows must still be listed: {body}"
    );

    // The sweep itself committed (task terminal-frozen); only the frame is lost.
    let t = inner.get_node_task(&task.id).await.unwrap().unwrap();
    assert_eq!(t.status.as_str(), "error");
    assert_eq!(t.error.as_deref(), Some("node lost"));
    let frames = inner.events_after(&task.session_id, -1).await.unwrap();
    assert!(
        frames.is_empty(),
        "append_events failed ⇒ the closure frame is absent, not half-written"
    );
}
