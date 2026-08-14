//! P1-5 TOCTOU regression: when a drain starts between `post_agent`/`post_model`'s
//! initial `draining` check and its `update_session` write, the handler must roll
//! back the meta change and return 409 — never persisting a divergent agent/model
//! nor leaking a runtime override.
//!
//! We simulate the race with a `DrainFlippingStore` wrapper that flips the
//! session handle's `draining` flag to `true` INSIDE `update_session` (the exact
//! TOCTOU window the re-check guards), then delegates the real write to the
//! in-memory libsql store. The handler's post-write re-check observes the drain
//! and rolls back.

#![allow(dead_code)]

use std::sync::atomic::Ordering;
use std::sync::Arc;

use async_trait::async_trait;
use axum::response::IntoResponse;
use opencoder_store::{
    Delivery, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput,
    SessionListItem, SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};
use opencoder_web::handle::SessionHandle;

/// Wraps a real store and delegates everything EXCEPT `update_session`, which
/// flips the bound session handle's `draining` flag to `true` BEFORE delegating
/// the write — reproducing a drain starting mid-write (the TOCTOU window).
/// With `fail_get = true`, `get_session` errors instead, reproducing the
/// pre-write rollback-capture read failing.
struct DrainFlippingStore {
    inner: Arc<dyn Store>,
    handle: Arc<SessionHandle>,
    fail_get: bool,
}

#[async_trait]
impl Store for DrainFlippingStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create_session(&self, meta: &SessionMeta) -> anyhow::Result<()> {
        self.inner.create_session(meta).await
    }
    async fn get_session(&self, id: &str) -> anyhow::Result<Option<SessionMeta>> {
        if self.fail_get {
            anyhow::bail!("simulated get_session failure");
        }
        self.inner.get_session(id).await
    }
    async fn list_sessions(
        &self,
        f: &SessionFilter,
    ) -> anyhow::Result<Vec<SessionListItem>> {
        self.inner.list_sessions(f).await
    }
    async fn update_session(
        &self,
        id: &str,
        patch: &SessionPatch,
    ) -> anyhow::Result<()> {
        // Simulate a drain starting DURING the write (the TOCTOU window).
        self.handle.draining.store(true, Ordering::SeqCst);
        self.inner.update_session(id, patch).await
    }
    async fn delete_session(&self, id: &str) -> anyhow::Result<()> {
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, keep: &str) -> anyhow::Result<u64> {
        self.inner.clear_other_sessions(keep).await
    }
    async fn append_message(
        &self,
        sid: &str,
        m: &opencoder_core::message::Message,
    ) -> anyhow::Result<i64> {
        self.inner.append_message(sid, m).await
    }
    async fn append_messages(
        &self,
        sid: &str,
        msgs: &[opencoder_core::message::Message],
    ) -> anyhow::Result<Vec<i64>> {
        self.inner.append_messages(sid, msgs).await
    }
    async fn load_messages(
        &self,
        sid: &str,
    ) -> anyhow::Result<Vec<opencoder_core::message::Message>> {
        self.inner.load_messages(sid).await
    }
    async fn last_message_seq(&self, sid: &str) -> anyhow::Result<i64> {
        self.inner.last_message_seq(sid).await
    }
    async fn admit_input(&self, input: &SessionInput) -> anyhow::Result<i64> {
        self.inner.admit_input(input).await
    }
    async fn pending_inputs(
        &self,
        sid: &str,
        d: Delivery,
    ) -> anyhow::Result<Vec<SessionInput>> {
        self.inner.pending_inputs(sid, d).await
    }
    async fn promote_inputs(
        &self,
        sid: &str,
        up_to: i64,
        d: Delivery,
    ) -> anyhow::Result<Vec<i64>> {
        self.inner.promote_inputs(sid, up_to, d).await
    }
    async fn promote_next_queued(&self, sid: &str) -> anyhow::Result<Option<i64>> {
        self.inner.promote_next_queued(sid).await
    }
    async fn claim_next_queue(
        &self,
        sid: &str,
    ) -> anyhow::Result<Option<(i64, SessionInput)>> {
        self.inner.claim_next_queue(sid).await
    }
    async fn delete_input(&self, input_id: i64) -> anyhow::Result<()> {
        self.inner.delete_input(input_id).await
    }
    async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> anyhow::Result<()> {
        self.inner.swap_input_order(sid, a, b).await
    }
    async fn append_events(
        &self,
        events: &[SessionEventRecord],
    ) -> anyhow::Result<Vec<i64>> {
        self.inner.append_events(events).await
    }
    async fn events_after(
        &self,
        sid: &str,
        after: i64,
    ) -> anyhow::Result<Vec<SessionEventRecord>> {
        self.inner.events_after(sid, after).await
    }
    async fn last_event_seq(&self, sid: &str) -> anyhow::Result<i64> {
        self.inner.last_event_seq(sid).await
    }
    async fn create_subagent_task(
        &self,
        r: &SubagentTaskRecord,
    ) -> anyhow::Result<()> {
        self.inner.create_subagent_task(r).await
    }
    async fn complete_subagent_task(
        &self,
        id: &str,
        result: &str,
        ok: bool,
    ) -> anyhow::Result<()> {
        self.inner.complete_subagent_task(id, result, ok).await
    }
    async fn list_subagent_tasks(
        &self,
        sid: &str,
    ) -> anyhow::Result<Vec<SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(sid).await
    }
    async fn get_subagent_task(
        &self,
        id: &str,
    ) -> anyhow::Result<Option<SubagentTaskRecord>> {
        self.inner.get_subagent_task(id).await
    }
    async fn cancel_subagent_task(&self, id: &str) -> anyhow::Result<()> {
        self.inner.cancel_subagent_task(id).await
    }
}

/// Build an AppState whose store flips `draining` during every `update_session`.
/// The handle for `sid` is pre-inserted so the handler finds a live, non-draining
/// handle on entry, then observes the drain on its post-write re-check.
async fn state_with_drain_flip(sid: &str) -> Arc<opencoder_web::AppState> {
    state_with_drain_flip_inner(sid, false).await.0
}

/// Same as [`state_with_drain_flip`], plus `get_session` failing so the
/// handler's rollback-capture read yields no old row. Returns the underlying
/// real store too — assertions must bypass the failing wrapper.
async fn state_with_drain_flip_failing_get(
    sid: &str,
) -> (Arc<opencoder_web::AppState>, Arc<LibsqlStore>) {
    state_with_drain_flip_inner(sid, true).await
}

async fn state_with_drain_flip_inner(
    sid: &str,
    fail_get: bool,
) -> (Arc<opencoder_web::AppState>, Arc<LibsqlStore>) {
    let real = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let inner: Arc<dyn Store> = real.clone();
    let handle = SessionHandle::new(); // draining starts false
    let handles = opencoder_web::handle::new_handle_map();
    handles.lock().await.insert(sid.to_string(), handle.clone());
    let store: Arc<dyn Store> = Arc::new(DrainFlippingStore {
        inner,
        handle,
        fail_get,
    });
    (
        Arc::new(opencoder_web::AppState {
            client_override: None,
            store,
            workdir: std::env::temp_dir(),
            handles,
        }),
        real,
    )
}

/// Seed a session row (agent "act", model "m").
async fn seed(state: &opencoder_web::AppState, sid: &str) {
    seed_as(state, sid, Some("act"), Some("m")).await;
}

/// Seed a session row with explicit (possibly NULL) agent/model.
async fn seed_as(state: &opencoder_web::AppState, sid: &str, agent: Option<&str>, model: Option<&str>) {
    state
        .store
        .create_session(&SessionMeta {
            id: sid.to_string(),
            title: None,
            agent: agent.map(String::from),
            model: model.map(String::from),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();
}

/// Read back the persisted meta for a session.
async fn meta(state: &opencoder_web::AppState, sid: &str) -> SessionMeta {
    state.store.get_session(sid).await.unwrap().unwrap()
}

/// Decode a handler response into (status, json).
async fn decode(
    resp: axum::response::Response,
) -> (axum::http::StatusCode, serde_json::Value) {
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    (status, v)
}

/// P1-5 (agent): a drain starting during `update_session` must roll back the
/// agent switch and return 409, leaving meta + runtime override untouched.
#[tokio::test]
async fn post_agent_rolls_back_on_toctou_drain_start() {
    let state = state_with_drain_flip("s1").await;
    seed(&state, "s1").await;
    assert_eq!(meta(&state, "s1").await.agent.as_deref(), Some("act"));

    let resp = opencoder_web::api::post_agent(
        axum::extract::State(state.clone()),
        axum::extract::Path("s1".to_string()),
        axum::Json(opencoder_web::api::SwitchBody {
            value: "explore".into(),
        }),
    )
    .await
    .into_response();

    let (status, v) = decode(resp).await;
    assert_eq!(
        status,
        axum::http::StatusCode::CONFLICT,
        "TOCTOU drain must yield 409, not 200; body: {v}"
    );
    assert_eq!(v["ok"], false, "rollback path must signal ok:false");
    assert_eq!(
        v.get("error").and_then(|s| s.as_str()).unwrap_or(""),
        "agent switch refused: drain started during write",
        "must report the TOCTOU rollback reason"
    );

    // Meta must be restored to its pre-switch value.
    assert_eq!(
        meta(&state, "s1").await.agent.as_deref(),
        Some("act"),
        "agent must be rolled back to the original value"
    );

    // Runtime override must NOT have been applied (handler returns before
    // touching overrides on the rollback path).
    let map = state.handles.lock().await;
    let o = map.get("s1").unwrap().overrides.lock().await;
    assert!(
        o.agent.is_none(),
        "runtime agent override must not leak after TOCTOU rollback"
    );
}

/// P1-5 (model): a drain starting during `update_session` must roll back the
/// model switch and return 409, leaving meta + runtime override untouched.
#[tokio::test]
async fn post_model_rolls_back_on_toctou_drain_start() {
    let state = state_with_drain_flip("s2").await;
    seed(&state, "s2").await;
    assert_eq!(meta(&state, "s2").await.model.as_deref(), Some("m"));

    let resp = opencoder_web::api::post_model(
        axum::extract::State(state.clone()),
        axum::extract::Path("s2".to_string()),
        axum::Json(opencoder_web::api::ModelBody {
            value: "new-model".into(),
            persist_default: false,
        }),
    )
    .await
    .into_response();

    let (status, v) = decode(resp).await;
    assert_eq!(
        status,
        axum::http::StatusCode::CONFLICT,
        "TOCTOU drain must yield 409, not 200; body: {v}"
    );
    assert_eq!(v["ok"], false, "rollback path must signal ok:false");
    assert_eq!(
        v.get("error").and_then(|s| s.as_str()).unwrap_or(""),
        "model switch refused: drain started during write",
        "must report the TOCTOU rollback reason"
    );

    // Meta must be restored to its pre-switch value.
    assert_eq!(
        meta(&state, "s2").await.model.as_deref(),
        Some("m"),
        "model must be rolled back to the original value"
    );

    // Runtime override must NOT have been applied.
    let map = state.handles.lock().await;
    let o = map.get("s2").unwrap().overrides.lock().await;
    assert!(
        o.model.is_none(),
        "runtime model override must not leak after TOCTOU rollback"
    );
}

/// P1-5 (agent, NULL old value): when the pre-switch agent was NULL — the
/// common case for sessions created without an explicit agent — the rollback
/// must CLEAR the column. The previous `agent: old.agent.clone()` patch built
/// `agent: None`, a silent no-op, so the refused switch stayed persisted.
#[tokio::test]
async fn post_agent_rolls_back_null_agent_by_clearing() {
    let state = state_with_drain_flip("s3").await;
    seed_as(&state, "s3", None, Some("m")).await;
    assert_eq!(meta(&state, "s3").await.agent, None);

    let resp = opencoder_web::api::post_agent(
        axum::extract::State(state.clone()),
        axum::extract::Path("s3".to_string()),
        axum::Json(opencoder_web::api::SwitchBody {
            value: "explore".into(),
        }),
    )
    .await
    .into_response();

    let (status, v) = decode(resp).await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT);
    assert_eq!(v["ok"], false);
    assert_eq!(
        meta(&state, "s3").await.agent, None,
        "refused switch must not persist: NULL old agent must be restored as NULL"
    );
    let map = state.handles.lock().await;
    let o = map.get("s3").unwrap().overrides.lock().await;
    assert!(o.agent.is_none(), "runtime agent override must not leak");
}

/// P1-5 (model, NULL old value): same contract for the model column.
#[tokio::test]
async fn post_model_rolls_back_null_model_by_clearing() {
    let state = state_with_drain_flip("s4").await;
    seed_as(&state, "s4", Some("act"), None).await;
    assert_eq!(meta(&state, "s4").await.model, None);

    let resp = opencoder_web::api::post_model(
        axum::extract::State(state.clone()),
        axum::extract::Path("s4".to_string()),
        axum::Json(opencoder_web::api::ModelBody {
            value: "new-model".into(),
            persist_default: false,
        }),
    )
    .await
    .into_response();

    let (status, v) = decode(resp).await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT);
    assert_eq!(v["ok"], false);
    assert_eq!(
        meta(&state, "s4").await.model, None,
        "refused switch must not persist: NULL old model must be restored as NULL"
    );
    let map = state.handles.lock().await;
    let o = map.get("s4").unwrap().overrides.lock().await;
    assert!(o.model.is_none(), "runtime model override must not leak");
}

/// P1-5 (agent, capture read failed): when the pre-write `get_session` errors,
/// no old row is captured. The rollback must still act (best-effort clear —
/// a drain running implies the row exists) instead of skipping the write,
/// which previously left the refused switch persisted.
#[tokio::test]
async fn post_agent_clears_agent_when_capture_read_failed() {
    let (state, real) = state_with_drain_flip_failing_get("s5").await;
    seed_as(&state, "s5", Some("act"), Some("m")).await;
    assert_eq!(real.get_session("s5").await.unwrap().unwrap().agent.as_deref(), Some("act"));

    let resp = opencoder_web::api::post_agent(
        axum::extract::State(state.clone()),
        axum::extract::Path("s5".to_string()),
        axum::Json(opencoder_web::api::SwitchBody {
            value: "explore".into(),
        }),
    )
    .await
    .into_response();

    let (status, v) = decode(resp).await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT);
    assert_eq!(v["ok"], false);
    assert_eq!(
        real.get_session("s5").await.unwrap().unwrap().agent, None,
        "failed capture must still roll back: best-effort clear to NULL"
    );
}

/// P1-5 (model, capture read failed): same best-effort clear for the model
/// column.
#[tokio::test]
async fn post_model_clears_model_when_capture_read_failed() {
    let (state, real) = state_with_drain_flip_failing_get("s6").await;
    seed_as(&state, "s6", Some("act"), Some("m")).await;
    assert_eq!(real.get_session("s6").await.unwrap().unwrap().model.as_deref(), Some("m"));

    let resp = opencoder_web::api::post_model(
        axum::extract::State(state.clone()),
        axum::extract::Path("s6".to_string()),
        axum::Json(opencoder_web::api::ModelBody {
            value: "new-model".into(),
            persist_default: false,
        }),
    )
    .await
    .into_response();

    let (status, v) = decode(resp).await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT);
    assert_eq!(v["ok"], false);
    assert_eq!(
        real.get_session("s6").await.unwrap().unwrap().model, None,
        "failed capture must still roll back: best-effort clear to NULL"
    );
}
