//! Regression contracts for three web-layer bug fixes. All drive the real
//! handlers against an in-memory store (no network).
//!
//! - Bug #8 `post_interrupt_no_handle_returns_ok_false`: POST /interrupt must
//!   answer `{"ok": false}` when there is no live handle (not 500/panic), and
//!   `{"ok": true}` once a handle exists and its cancel token fires.
//! - Bug #4 `create_session_returns_500_on_store_failure`: a Store write error
//!   on POST /sessions must surface as a structured 500 (not an unhandled
//!   error). Verified through a `FailingStore` wrapper that breaks only
//!   `create_session` and delegates everything else.
//! - Bug #1 `events_subscribe_first_no_loss_no_dup`: GET /events subscribes
//!   BEFORE replaying the persisted window, so a persisted+broadcast event is
//!   delivered exactly once (deduped) and a live-only event is never lost.

#![allow(dead_code)]

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use futures::StreamExt;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{
    EventKind, LibsqlStore, SessionEventRecord, SessionMeta, Store,
};
use opencoder_web::handle::SseEvt;
use serde_json::json;

/// Fresh in-memory AppState (handlers are called directly, no router).
async fn state() -> Arc<opencoder_web::AppState> {
    state_with_store(Arc::new(LibsqlStore::open_memory().await.unwrap())).await
}

/// AppState backed by a custom store (for the failing-store regression).
async fn state_with_store(store: Arc<dyn Store>) -> Arc<opencoder_web::AppState> {
    Arc::new(opencoder_web::AppState {
        client_override: None,
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
    })
}

/// Seed a session row (default agent "act", model "m").
async fn seed(state: &opencoder_web::AppState, sid: &str) {
    state
        .store
        .create_session(&SessionMeta {
            id: sid.to_string(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),
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
        })
        .await
        .unwrap();
}

/// Mock that completes a single assistant turn replying `text`.
fn mock_reply(text: &str) -> Arc<dyn ChatStream> {
    Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: None,
        }]),
    )
}

/// Poll until the session's drain is idle (`draining` reset).
async fn wait_idle(state: &opencoder_web::AppState, sid: &str) {
    for _ in 0..120 {
        let idle = state
            .handles
            .lock()
            .await
            .get(sid)
            .map(|h| !h.draining.load(Ordering::SeqCst))
            .unwrap_or(true);
        if idle {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("drain for {sid} never went idle");
}

/// Wraps a real store and delegates everything EXCEPT `create_session`, which
/// always fails. Mirrors the `CountingStore` delegation pattern in
/// `crates/session/src/event_sink.rs`.
struct FailingStore {
    inner: Arc<dyn Store>,
}

#[async_trait]
impl Store for FailingStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create_session(&self, _meta: &SessionMeta) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("simulated disk failure"))
    }
    async fn get_session(&self, id: &str) -> anyhow::Result<Option<SessionMeta>> {
        self.inner.get_session(id).await
    }
    async fn list_sessions(
        &self,
        f: &opencoder_store::SessionFilter,
    ) -> anyhow::Result<Vec<opencoder_store::SessionListItem>> {
        self.inner.list_sessions(f).await
    }
    async fn update_session(
        &self,
        id: &str,
        p: &opencoder_store::SessionPatch,
    ) -> anyhow::Result<()> {
        self.inner.update_session(id, p).await
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
    async fn admit_input(
        &self,
        input: &opencoder_store::SessionInput,
    ) -> anyhow::Result<i64> {
        self.inner.admit_input(input).await
    }
    async fn pending_inputs(
        &self,
        sid: &str,
        d: opencoder_store::Delivery,
    ) -> anyhow::Result<Vec<opencoder_store::SessionInput>> {
        self.inner.pending_inputs(sid, d).await
    }
    async fn promote_inputs(
        &self,
        sid: &str,
        up_to: i64,
        d: opencoder_store::Delivery,
    ) -> anyhow::Result<Vec<i64>> {
        self.inner.promote_inputs(sid, up_to, d).await
    }
    async fn promote_next_queued(&self, sid: &str) -> anyhow::Result<Option<i64>> {
        self.inner.promote_next_queued(sid).await
    }
    async fn claim_next_queue(
        &self,
        sid: &str,
    ) -> anyhow::Result<Option<(i64, opencoder_store::SessionInput)>> {
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
        r: &opencoder_store::SubagentTaskRecord,
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
    ) -> anyhow::Result<Vec<opencoder_store::SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(sid).await
    }
    async fn get_subagent_task(
        &self,
        id: &str,
    ) -> anyhow::Result<Option<opencoder_store::SubagentTaskRecord>> {
        self.inner.get_subagent_task(id).await
    }
    async fn cancel_subagent_task(&self, id: &str) -> anyhow::Result<()> {
        self.inner.cancel_subagent_task(id).await
    }
}

/// Bug #8: POST /interrupt must answer `{"ok": false}` (not error) when no
/// handle exists, and `{"ok": true}` once a live handle is present.
#[tokio::test]
async fn post_interrupt_no_handle_returns_ok_false() {
    let state = state().await;

    // No handle for "ghost" -> ok:false, not a 5xx.
    let resp = opencoder_web::api::post_interrupt(
        State(state.clone()),
        Path("ghost".to_string()),
    )
    .await
    .into_response();
    assert_eq!(
        resp.status(),
        axum::http::StatusCode::OK,
        "missing handle is not a server error"
    );
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], false, "missing handle must signal ok:false");
    assert!(
        v.get("error").is_some(),
        "missing handle must include an error message"
    );

    // Positive: a live (actively draining) handle flips the answer to ok:true.
    // A bare SessionHandle::new() has draining=false; an idle handle must NOT
    // report ok:true (Bug #2), so simulate a running drain here.
    let live = opencoder_web::handle::SessionHandle::new();
    live.draining.store(true, Ordering::SeqCst);
    state.handles.lock().await.insert("live".to_string(), live);
    let resp = opencoder_web::api::post_interrupt(
        State(state.clone()),
        Path("live".to_string()),
    )
    .await
    .into_response();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], true, "actively draining handle must signal ok:true");
}

/// Bug #4: a Store failure on POST /sessions must surface as a structured 500.
#[tokio::test]
async fn create_session_returns_500_on_store_failure() {
    let inner: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let failing: Arc<dyn Store> = Arc::new(FailingStore { inner });
    let state = state_with_store(failing).await;

    let resp = opencoder_web::api::create_session(State(state), None)
        .await
        .into_response();
    assert_eq!(
        resp.status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "store write failure must be a 500"
    );
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], false, "error body must signal ok:false");
    assert!(
        v["error"].as_str().unwrap_or_default().contains("disk failure"),
        "error must surface the store failure: {}",
        v["error"]
    );
}

/// Bug #1: GET /events subscribes BEFORE replay, so an event that is both
/// persisted and broadcast is delivered exactly once (deduped, not duplicated),
/// and a live-only event is never lost.
#[tokio::test]
async fn events_subscribe_first_no_loss_no_dup() {
    let state = state().await;
    let sid = "s1";
    seed(&state, sid).await;

    // (a) Persist E1 first. It will land in the replay window.
    let e1 = json!({ "test": "event1" });
    state
        .store
        .append_event(&SessionEventRecord {
            session_id: sid.into(),
            kind: EventKind::Done,
            payload: e1.clone(),
            ts: 1,
            seq: None,
            sse_kind: Some("done".into()),
        })
        .await
        .unwrap();

    // (b) Subscribe+replay via the handler. Internally this subscribes a live
    // receiver BEFORE running the replay query, then chains replay . live.
    let resp = opencoder_web::api::get_events(
        State(state.clone()),
        Path(sid.to_string()),
        Query(opencoder_web::api::EventsQuery { after: Some(0) }),
    )
    .await
    .into_response();

    // Grab the handle's tx (created by get_events) to broadcast.
    let tx = {
        let map = state.handles.lock().await;
        map.get(sid).unwrap().tx.clone()
    };

    // (c) Re-broadcast E1 (already persisted -> would duplicate without dedup).
    let _ = tx.send(SseEvt {
        kind: "done".into(),
        data: e1.clone(),
        ts: 2,
        seq: None,
    });
    // (d) Broadcast E2 (live-only, never persisted -> would be lost without
    // subscribe-first capturing it on the live receiver).
    let _ = tx.send(SseEvt {
        kind: "done".into(),
        data: json!({ "test": "event2" }),
        ts: 3,
        seq: None,
    });

    // (e) Consume the SSE bytes for a bounded window (broadcast streams stay
    // open forever, so rely on timeouts + a deadline).
    let mut stream = resp.into_body().into_data_stream();
    let mut text = String::new();
    let deadline = std::time::Instant::now() + Duration::from_millis(400);
    while std::time::Instant::now() < deadline {
        if let Ok(Some(Ok(bytes))) =
            tokio::time::timeout(Duration::from_millis(50), stream.next()).await
        {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }

    let e1_count = text.matches("event1").count();
    let e2_count = text.matches("event2").count();
    assert_eq!(
        e1_count, 1,
        "persisted+broadcast event must appear exactly once (deduped); got stream:\n{text}"
    );
    assert_eq!(
        e2_count, 1,
        "live-only event must appear exactly once (not lost); got stream:\n{text}"
    );
}
