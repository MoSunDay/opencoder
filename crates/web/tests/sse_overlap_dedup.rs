//! Regression test for **BUG 8**: the SSE `seen` content-dedup set was always
//! empty because `baseline = last_event_seq(...)` was computed AFTER the
//! `events_after(...)` query. `last_event_seq` returns the current max
//! persisted seq, so reading it after `events_after` guarantees `baseline >=
//! max(seq)`; the `seq > baseline` filter is then always false and `seen`
//! never seeds — tier-(2) content dedup of overlap-window events was dead code.
//!
//! This test models the subscribe/query overlap window DETERMINISTICALLY with a
//! Store wrapper that lazily persists an "overlap" event the first time
//! `events_after` runs (standing in for an event persisted by another task
//! between the baseline snapshot and the replay query). Because `last_event_seq`
//! delegates to the real store (which reflects actual persisted rows at call
//! time), the call ORDER in production code decides the outcome:
//!
//!  - FIXED (baseline before events_after): baseline sees only the base event
//!    (seq 1); the overlap event (seq 2) lands in the window, seeds `seen`, and
//!    its live broadcast is deduped -> delivered exactly once (replay only).
//!  - BUGGY (baseline after events_after): baseline sees seq 2; the overlap
//!    event fails `seq > baseline`, `seen` stays empty, and the live broadcast
//!    is delivered again -> the overlap payload appears twice.
//!
//! `overlap_count == 1` passes under the fix and fails (== 2) under the bug.
//! No real timing/race is involved.

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use futures::StreamExt;
use opencoder_core::Message;
use opencoder_store::{
    Delivery, EventKind, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput,
    SessionListItem, SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};
use opencoder_web::handle::SseEvt;
use serde_json::json;
use tokio::sync::Mutex;

/// Store wrapper that injects an "overlap-window" event the first time
/// `events_after` runs, modelling a concurrent persistence between the baseline
/// snapshot and the replay query. Every other method delegates to the inner
/// libsql store, so `last_event_seq` truthfully reports persisted state *at call
/// time* — which is what makes the production call order observable here.
struct OverlapStore {
    inner: Arc<LibsqlStore>,
    overlap: Mutex<Option<SessionEventRecord>>,
    seeded: AtomicBool,
}

#[async_trait]
impl Store for OverlapStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create_session(&self, meta: &SessionMeta) -> Result<()> {
        self.inner.create_session(meta).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        self.inner.get_session(id).await
    }
    async fn list_sessions(&self, f: &SessionFilter) -> Result<Vec<SessionListItem>> {
        self.inner.list_sessions(f).await
    }
    async fn update_session(&self, id: &str, patch: &SessionPatch) -> Result<()> {
        self.inner.update_session(id, patch).await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, keep: &str) -> Result<u64> {
        self.inner.clear_other_sessions(keep).await
    }
    async fn append_message(&self, sid: &str, msg: &Message) -> Result<i64> {
        self.inner.append_message(sid, msg).await
    }
    async fn append_messages(&self, sid: &str, msgs: &[Message]) -> Result<Vec<i64>> {
        self.inner.append_messages(sid, msgs).await
    }
    async fn load_messages(&self, sid: &str) -> Result<Vec<Message>> {
        self.inner.load_messages(sid).await
    }
    async fn last_message_seq(&self, sid: &str) -> Result<i64> {
        self.inner.last_message_seq(sid).await
    }
    async fn admit_input(&self, input: &SessionInput) -> Result<i64> {
        self.inner.admit_input(input).await
    }
    async fn pending_inputs(&self, sid: &str, d: Delivery) -> Result<Vec<SessionInput>> {
        self.inner.pending_inputs(sid, d).await
    }
    async fn promote_inputs(&self, sid: &str, up_to: i64, d: Delivery) -> Result<Vec<i64>> {
        self.inner.promote_inputs(sid, up_to, d).await
    }
    async fn promote_next_queued(&self, sid: &str) -> Result<Option<i64>> {
        self.inner.promote_next_queued(sid).await
    }
    async fn claim_next_queue(&self, sid: &str) -> Result<Option<(i64, SessionInput)>> {
        self.inner.claim_next_queue(sid).await
    }
    async fn delete_input(&self, input_id: i64) -> Result<()> {
        self.inner.delete_input(input_id).await
    }
    async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> Result<()> {
        self.inner.swap_input_order(sid, a, b).await
    }
    async fn append_events(&self, events: &[SessionEventRecord]) -> Result<Vec<i64>> {
        self.inner.append_events(events).await
    }
    async fn events_after(&self, sid: &str, after: i64) -> Result<Vec<SessionEventRecord>> {
        // Inject the overlap event on the first query, simulating an event
        // persisted between the baseline snapshot and this query.
        if !self.seeded.swap(true, Ordering::SeqCst) {
            if let Some(rec) = self.overlap.lock().await.take() {
                self.inner.append_event(&rec).await?;
            }
        }
        self.inner.events_after(sid, after).await
    }
    async fn last_event_seq(&self, sid: &str) -> Result<i64> {
        // Deliberately delegate: returns the persisted max AT CALL TIME, so the
        // production call order (baseline vs events_after) is observable.
        self.inner.last_event_seq(sid).await
    }
    async fn create_subagent_task(&self, rec: &SubagentTaskRecord) -> Result<()> {
        self.inner.create_subagent_task(rec).await
    }
    async fn complete_subagent_task(&self, task_id: &str, result: &str, ok: bool) -> Result<()> {
        self.inner.complete_subagent_task(task_id, result, ok).await
    }
    async fn list_subagent_tasks(&self, parent: &str) -> Result<Vec<SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(parent).await
    }
    async fn get_subagent_task(&self, task_id: &str) -> Result<Option<SubagentTaskRecord>> {
        self.inner.get_subagent_task(task_id).await
    }
    async fn cancel_subagent_task(&self, task_id: &str) -> Result<()> {
        self.inner.cancel_subagent_task(task_id).await
    }
}

async fn seed(state: &opencoder_web::AppState, sid: &str) {
    state
        .store
        .create_session(&SessionMeta {
            id: sid.into(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),

            autopilot_mode: None,
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
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await
        .unwrap();
}

/// Unique marker so the dedup count is unambiguous in the streamed bytes.
const OVERLAP_MARKER: &str = "__overlap_window_seed__";

#[tokio::test]
async fn overlap_window_event_is_deduped_once() {
    let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let sid = "ov";
    let store: Arc<dyn Store> = Arc::new(OverlapStore {
        inner: inner.clone(),
        overlap: Mutex::new(Some(SessionEventRecord {
            session_id: sid.into(),
            kind: EventKind::Step,
            payload: json!({ "k": OVERLAP_MARKER }),
            ts: 2,
            seq: None,
            sse_kind: Some("status".into()),
        })),
        seeded: AtomicBool::new(false),
    });
    let state = Arc::new(opencoder_web::AppState {
        client_override: None,
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
    });
    seed(&state, sid).await;

    // (a) A base event persisted BEFORE the handler — the baseline must include
    // it (seq 1), so the overlap event (seq 2) genuinely exceeds the baseline.
    state
        .store
        .append_event(&SessionEventRecord {
            session_id: sid.into(),
            kind: EventKind::Step,
            payload: json!({ "k": "base" }),
            ts: 1,
            seq: None,
            sse_kind: Some("status".into()),
        })
        .await
        .unwrap();

    // (b) Run the SSE handler. It subscribes, snapshots baseline (=1), then
    // queries events_after — which lazily persists the overlap event (seq 2),
    // landing it in the overlap window so it seeds `seen`.
    let resp = opencoder_web::api::get_events(
        State(state.clone()),
        Path(sid.to_string()),
        Query(opencoder_web::api::EventsQuery { after: Some(0) }),
        axum::http::HeaderMap::new(),
    )
    .await
    .into_response();

    // Grab the broadcast tx created by get_events so we can emit a live event.
    let tx = {
        let map = state.handles.lock().await;
        map.get(sid).unwrap().tx.clone()
    };

    // (c) Broadcast the overlap event LIVE (seq: None) — the duplicate the
    // content-dedup set must swallow under the fix.
    let _ = tx.send(SseEvt {
        kind: "status".into(),
        data: json!({ "k": OVERLAP_MARKER }),
        ts: 3,
        seq: None,
    });

    // (d) Drain the SSE bytes for a bounded window (the live stream never ends).
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

    // The overlap payload must appear EXACTLY once (replay). Under the bug
    // `seen` was empty, the live broadcast was delivered again (== 2).
    let overlap_count = text.matches(OVERLAP_MARKER).count();
    assert_eq!(
        overlap_count, 1,
        "overlap-window event was not deduped (BUG 8 regression): expected \
         exactly 1 delivery of the overlap payload (replay only), got \
         {overlap_count}:\n{text}"
    );
}
