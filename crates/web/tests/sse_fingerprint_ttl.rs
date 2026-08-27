//! Regression test for **P2-4 (SSE fingerprint TTL)**: the tier-(2) content
//! dedup set `seen` in `get_events` used to live for the whole stream. It is
//! seeded from the subscribe→query overlap window and each fingerprint is
//! consumed on first match, but any fingerprint NOT matched by a duplicate
//! broadcast stayed in the set forever. A *later* live event whose
//! (kind, data) happened to collide with an overlap-window fingerprint was
//! silently dropped for the entire stream lifetime.
//!
//! Fix under test: the first live `done` event that PASSES the dedup check
//! clears `seen` — `done` deterministically closes a run, so after the first
//! forwarded `done` no further live event can belong to the overlap window
//! and content collisions are genuine new events that must be forwarded.
//!
//! Scenario (deterministic, same Store-wrapper trick as `sse_overlap_dedup`):
//!  1. base event persisted before the handler (seq 1);
//!  2. the handler snapshots baseline (=1), then `events_after` lazily
//!     persists the overlap event (seq 2) → seeds `seen` with fingerprint X;
//!  3. live: a `done` broadcast (passes dedup — clears `seen` under the fix);
//!  4. live: a fresh event whose content is exactly X.
//!
//! FIXED: step 4 is forwarded → X is delivered twice (replay + live).
//! BUGGY (no TTL): step 4 collides with the still-seeded X and is dropped →
//! X is delivered once → the assertion `count == 2` fails.

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
/// `events_after` runs, modelling an event persisted by a concurrent task
/// between the handler's baseline snapshot and its replay query. Everything
/// else delegates to the inner libsql store.
struct OverlapSeedStore {
    inner: Arc<LibsqlStore>,
    overlap: Mutex<Option<SessionEventRecord>>,
    seeded: AtomicBool,
}

#[async_trait]
impl Store for OverlapSeedStore {
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
    async fn pending_inputs(&self, sid: &str, delivery: Delivery) -> Result<Vec<SessionInput>> {
        self.inner.pending_inputs(sid, delivery).await
    }
    async fn promote_inputs(
        &self,
        sid: &str,
        up_to_admitted_seq: i64,
        delivery: Delivery,
    ) -> Result<Vec<i64>> {
        self.inner
            .promote_inputs(sid, up_to_admitted_seq, delivery)
            .await
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
        // Persist the overlap event on the first query: the handler already
        // snapshotted the baseline, so this event lands in the overlap window
        // (seq > baseline) and seeds the tier-(2) `seen` fingerprint set.
        if !self.seeded.swap(true, Ordering::SeqCst) {
            if let Some(rec) = self.overlap.lock().await.take() {
                self.inner.append_event(&rec).await?;
            }
        }
        self.inner.events_after(sid, after).await
    }
    async fn last_event_seq(&self, sid: &str) -> Result<i64> {
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

/// Unique marker so the dedup/delivery count is unambiguous in the bytes.
const TTL_MARKER: &str = "__fingerprint_ttl_seed__";

#[tokio::test]
async fn seen_fingerprints_expire_at_first_forwarded_done() {
    let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let sid = "ttl";
    let store: Arc<dyn Store> = Arc::new(OverlapSeedStore {
        inner: inner.clone(),
        overlap: Mutex::new(Some(SessionEventRecord {
            session_id: sid.into(),
            kind: EventKind::Step,
            payload: json!({ "k": TTL_MARKER }),
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

    // (a) A base event persisted BEFORE the handler: the baseline (=1) is
    // strictly below the overlap event's seq (2), so the overlap event seeds
    // `seen` with ("status", {"k": TTL_MARKER}).
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

    // (b) Run the SSE handler: subscribe → baseline(=1) → events_after
    // (lazily persists the overlap event, seq 2 → seeds `seen`).
    let resp = opencoder_web::api::get_events(
        State(state.clone()),
        Path(sid.to_string()),
        Query(opencoder_web::api::EventsQuery { after: Some(0) }),
        axum::http::HeaderMap::new(),
    )
    .await
    .into_response();

    let tx = {
        let map = state.handles.lock().await;
        map.get(sid).unwrap().tx.clone()
    };

    // (c) Live `done` first: it passes the dedup check (never seeded) and,
    // under the fix, clears `seen` — the overlap window is deterministically
    // over once a run's `done` is forwarded.
    let _ = tx.send(SseEvt {
        kind: "done".into(),
        data: json!({}),
        ts: 3,
        seq: None,
    });
    // (d) Then a genuine NEW live event whose content collides with the
    // overlap fingerprint. Post-fix it must be forwarded; pre-fix the stale
    // fingerprint in `seen` eats it.
    let _ = tx.send(SseEvt {
        kind: "status".into(),
        data: json!({ "k": TTL_MARKER }),
        ts: 4,
        seq: None,
    });

    // Drain the SSE bytes for a bounded window (live stream never ends).
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

    // The live `done` must be forwarded (sanity: the TTL trigger fired).
    assert!(
        text.contains("event: done"),
        "live done must be forwarded; stream:\n{text}"
    );
    // X delivered exactly twice: once from the replay window, once live.
    // Pre-fix the live copy was swallowed by the stale overlap fingerprint.
    let marker_count = text.matches(TTL_MARKER).count();
    assert_eq!(
        marker_count, 2,
        "after the first forwarded live done, a content collision with the \
         overlap window must be forwarded as a NEW event (P2-4 TTL): expected \
         2 deliveries of the marker (replay + live), got {marker_count}:\n{text}"
    );
}
