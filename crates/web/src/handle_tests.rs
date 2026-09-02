//! Unit tests for the web session-handle module (drain guard, subscriber
//! eviction, release semantics). Extracted from `handle.rs` via
//! `#[path]` so the runtime file stays under the repo line cap.
use super::*;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn release_subscriber_evicts_creator_handle_when_last_and_idle() {
    let handles = new_handle_map();
    let id = "sess-evict".to_string();
    let h = SessionHandle::new();
    h.subscribers.store(1, Ordering::SeqCst);
    h.draining.store(false, Ordering::SeqCst);
    handles.lock().await.insert(id.clone(), h);

    release_events_subscriber(handles.clone(), id.clone(), true);

    // The eviction runs in a spawned task; poll until it settles.
    for _ in 0..200 {
        if !handles.lock().await.contains_key(&id) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    assert!(
        !handles.lock().await.contains_key(&id),
        "creator handle should be evicted when last subscriber leaves and idle"
    );
}

#[tokio::test]
async fn release_subscriber_keeps_handle_while_draining() {
    let handles = new_handle_map();
    let id = "sess-drain".to_string();
    let h = SessionHandle::new();
    h.subscribers.store(1, Ordering::SeqCst);
    h.draining.store(true, Ordering::SeqCst);
    handles.lock().await.insert(id.clone(), h);

    release_events_subscriber(handles.clone(), id.clone(), true);
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(
        handles.lock().await.contains_key(&id),
        "handle must survive while a drain is running"
    );
}

#[tokio::test]
async fn release_subscriber_waits_for_idle_lifecycle_mutation() {
    let handles = new_handle_map();
    let id = "sess-lifecycle".to_string();
    let handle = SessionHandle::new();
    handle.subscribers.store(1, Ordering::SeqCst);
    handles.lock().await.insert(id.clone(), handle.clone());
    let lifecycle = handle.lifecycle.clone().lock_owned().await;

    release_events_subscriber(handles.clone(), id.clone(), true);
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(
        handles.lock().await.contains_key(&id),
        "eviction must not replace a handle during a lifecycle mutation"
    );

    drop(lifecycle);
    for _ in 0..200 {
        if !handles.lock().await.contains_key(&id) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    assert!(
        !handles.lock().await.contains_key(&id),
        "last-subscriber eviction should resume after the mutation"
    );
}

#[tokio::test]
async fn release_subscriber_keeps_handle_while_others_remain() {
    let handles = new_handle_map();
    let id = "sess-guest".to_string();
    let h = SessionHandle::new();
    // Two subscribers; a single non-creator release (prev == 2) is NOT the
    // last subscriber, so the handle must survive.
    h.subscribers.store(2, Ordering::SeqCst);
    h.draining.store(false, Ordering::SeqCst);
    handles.lock().await.insert(id.clone(), h);

    release_events_subscriber(handles.clone(), id.clone(), false);
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(
        handles.lock().await.contains_key(&id),
        "handle must survive when a non-creator leaves but another subscriber remains"
    );
}

// Bug #4: eviction must not depend on the `created` flag. If the creator
// disconnects first while a second (non-creator) subscriber remains, that
// second subscriber must still evict the handle when it becomes the last
// one leaving. The old `created &&` condition skipped eviction for the
// non-creator, leaking the handle forever.
#[tokio::test]
async fn session_handle_evicted_when_creator_leaves_first() {
    let handles = new_handle_map();
    let id = "test-session".to_string();

    // Simulate subscriber A creating the handle (creator).
    {
        let mut map = handles.lock().await;
        let handle = map.entry(id.clone()).or_insert_with(SessionHandle::new);
        handle.subscribers.fetch_add(1, Ordering::SeqCst);
    }

    // Simulate subscriber B joining (non-creator).
    {
        let mut map = handles.lock().await;
        let handle = map.entry(id.clone()).or_insert_with(SessionHandle::new);
        handle.subscribers.fetch_add(1, Ordering::SeqCst);
    }

    // Creator A leaves first (created=true, prev=2 → not the last, kept).
    release_events_subscriber(handles.clone(), id.clone(), true);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    {
        let map = handles.lock().await;
        assert!(
            map.contains_key(&id),
            "handle should survive when creator leaves but another subscriber remains"
        );
    }

    // Subscriber B leaves last (created=false, prev=1 → must be evicted).
    release_events_subscriber(handles.clone(), id.clone(), false);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    {
        let map = handles.lock().await;
        assert!(
            !map.contains_key(&id),
            "handle should be evicted when last subscriber leaves, even if not the creator"
        );
    }
}

// Bug: `release_events_subscriber` looks the handle up by session id, so a
// release aimed at an OLD (already-removed) instance can land on a freshly
// created same-id handle whose counter is 0. A blind `fetch_sub` wraps to
// `usize::MAX`, corrupting the count and disabling last-subscriber
// eviction forever. The decrement must saturate at 0.
#[tokio::test]
async fn release_subscriber_does_not_underflow_fresh_instance() {
    let handles = new_handle_map();
    let id = "sess-underflow".to_string();
    let h = SessionHandle::new();
    // Fresh instance: no subscriber ever attached (count 0), not draining.
    h.subscribers.store(0, Ordering::SeqCst);
    h.draining.store(false, Ordering::SeqCst);
    handles.lock().await.insert(id.clone(), h.clone());

    // A stale release for the old same-id instance fires on this handle.
    release_events_subscriber(handles.clone(), id.clone(), true);

    // The release runs in a spawned task; poll until it settles (counter
    // either changed or the sleep guarantees the task ran).
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        h.subscribers.load(Ordering::SeqCst),
        0,
        "subscriber counter must saturate at 0, not wrap to usize::MAX"
    );
    assert!(
        handles.lock().await.contains_key(&id),
        "a zero-count release must not evict the handle"
    );
}

/// P1: a run that ends in `Err` WITHOUT the runner having emitted its own
/// `Error` event must still produce a terminal error frame (broadcast +
/// persisted) — otherwise the SSE stream hangs open forever.
#[tokio::test]
async fn run_err_without_runner_error_emits_terminal_error_frame() {
    let store: Arc<dyn Store> =
        Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "term-err".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let handle = SessionHandle::new();
    let mut rx = handle.tx.subscribe();

    let err: Result<()> = Err(anyhow::anyhow!("control command apply failed"));
    ensure_run_error_frame(&store, &handle, "term-err", &err, false).await;

    let evt = rx
        .try_recv()
        .expect("terminal error frame must be broadcast");
    assert_eq!(evt.kind, "error");
    assert!(
        evt.data["error"]
            .as_str()
            .unwrap()
            .contains("control command apply failed"),
        "frame must carry the run failure reason"
    );
    let rows = store.events_after("term-err", 0).await.unwrap();
    assert_eq!(
        rows.iter().filter(|r| r.kind == EventKind::Error).count(),
        1,
        "the synthetic error must be persisted exactly once"
    );
}

/// The suppression half of the contract: when the runner already emitted its
/// own Error (LLM failure path), the drain must NOT double-report — exactly
/// one Error stays on the wire and in the store (pinned integration-side by
/// drain_no_restart_on_error's exactly-one assertion).
#[tokio::test]
async fn run_err_with_runner_error_is_not_duplicated() {
    let store: Arc<dyn Store> =
        Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "term-err-dup".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let handle = SessionHandle::new();
    let mut rx = handle.tx.subscribe();

    let err: Result<()> = Err(anyhow::anyhow!("simulated llm outage"));
    ensure_run_error_frame(&store, &handle, "term-err-dup", &err, true).await;

    assert!(
        rx.try_recv().is_err(),
        "no extra frame when the runner already emitted its own Error"
    );
    let rows = store.events_after("term-err-dup", 0).await.unwrap();
    assert_eq!(
        rows.iter().filter(|r| r.kind == EventKind::Error).count(),
        0
    );
}

/// Ok runs never produce a synthetic error frame.
#[tokio::test]
async fn run_ok_emits_no_error_frame() {
    let store: Arc<dyn Store> =
        Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "term-ok".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let handle = SessionHandle::new();
    let mut rx = handle.tx.subscribe();

    let ok: Result<()> = Ok(());
    ensure_run_error_frame(&store, &handle, "term-ok", &ok, false).await;
    assert!(rx.try_recv().is_err());
    assert_eq!(store.events_after("term-ok", 0).await.unwrap().len(), 0);
}

/// Store wrapper that makes the persist/live-send ORDER of
/// `broadcast_persist_event` observable: at `append_event` time it probes the
/// parked SSE receiver. If the live frame is already queued, the broadcast
/// beat the persist (the old, buggy order). Everything else delegates to the
/// inner libsql store, so persisted state stays truthful.
struct OrderSpyStore {
    inner: Arc<opencoder_store::LibsqlStore>,
    rx: std::sync::Mutex<Option<broadcast::Receiver<SseEvt>>>,
    live_first: Arc<std::sync::atomic::AtomicBool>,
    fail_append: bool,
}

#[async_trait::async_trait]
impl Store for OrderSpyStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create_session(&self, meta: &opencoder_store::SessionMeta) -> Result<()> {
        self.inner.create_session(meta).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<opencoder_store::SessionMeta>> {
        self.inner.get_session(id).await
    }
    async fn list_sessions(
        &self,
        f: &opencoder_store::SessionFilter,
    ) -> Result<Vec<opencoder_store::SessionListItem>> {
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
    async fn append_message(&self, sid: &str, msg: &opencoder_core::Message) -> Result<i64> {
        self.inner.append_message(sid, msg).await
    }
    async fn append_messages(
        &self,
        sid: &str,
        msgs: &[opencoder_core::Message],
    ) -> Result<Vec<i64>> {
        self.inner.append_messages(sid, msgs).await
    }
    async fn load_messages(&self, sid: &str) -> Result<Vec<opencoder_core::Message>> {
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
    async fn append_event(&self, event: &SessionEventRecord) -> Result<i64> {
        // Probe: was the live frame broadcast BEFORE the persist?
        if let Some(mut rx) = self.rx.lock().unwrap().take() {
            if rx.try_recv().is_ok() {
                self.live_first
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        if self.fail_append {
            return Err(anyhow::anyhow!("simulated store outage"));
        }
        self.inner.append_event(event).await
    }
    async fn events_after(&self, sid: &str, after: i64) -> Result<Vec<SessionEventRecord>> {
        self.inner.events_after(sid, after).await
    }
    async fn last_event_seq(&self, sid: &str) -> Result<i64> {
        self.inner.last_event_seq(sid).await
    }
    async fn create_subagent_task(&self, rec: &opencoder_store::SubagentTaskRecord) -> Result<()> {
        self.inner.create_subagent_task(rec).await
    }
    async fn complete_subagent_task(&self, task_id: &str, result: &str, ok: bool) -> Result<()> {
        self.inner.complete_subagent_task(task_id, result, ok).await
    }
    async fn list_subagent_tasks(
        &self,
        parent: &str,
    ) -> Result<Vec<opencoder_store::SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(parent).await
    }
    async fn get_subagent_task(
        &self,
        task_id: &str,
    ) -> Result<Option<opencoder_store::SubagentTaskRecord>> {
        self.inner.get_subagent_task(task_id).await
    }
    async fn cancel_subagent_task(&self, task_id: &str) -> Result<()> {
        self.inner.cancel_subagent_task(task_id).await
    }
}

/// F2: persist-then-broadcast order. A subscriber that arrives between the
/// append and the live send must find the row via replay (its overlap
/// fingerprint then swallows the live copy in `sse_dedup`). Under the old
/// broadcast-first order that subscriber got NEITHER copy: at persist time the
/// live frame had already flown by — which is exactly what the probe asserts
/// against here.
#[tokio::test]
async fn broadcast_persists_before_live_send() {
    let inner = Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
    let live_first = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = SessionHandle::new();
    let spy = OrderSpyStore {
        inner: inner.clone(),
        rx: std::sync::Mutex::new(Some(handle.tx.subscribe())),
        live_first: live_first.clone(),
        fail_append: false,
    };
    let store: Arc<dyn Store> = Arc::new(spy);
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "bp-order".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut rx = handle.tx.subscribe();
    broadcast_persist_event(
        &store,
        &handle,
        "bp-order",
        SessionEvent::Error("switch failed".into()),
    )
    .await;

    assert!(
        !live_first.load(std::sync::atomic::Ordering::SeqCst),
        "the live broadcast must not precede the store append"
    );
    let evt = rx
        .try_recv()
        .expect("live frame delivered after the persist");
    assert_eq!(evt.kind, "error");
    let rows = store.events_after("bp-order", 0).await.unwrap();
    assert_eq!(
        rows.iter().filter(|r| r.kind == EventKind::Error).count(),
        1
    );
}

/// The warn-only half of the contract survives the reorder: a store outage
/// must not swallow the live frame (transient events degrade gracefully).
#[tokio::test]
async fn broadcast_persist_failure_still_delivers_live() {
    let inner = Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
    let handle = SessionHandle::new();
    let spy = OrderSpyStore {
        inner,
        rx: std::sync::Mutex::new(None),
        live_first: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        fail_append: true,
    };
    let store: Arc<dyn Store> = Arc::new(spy);
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "bp-fail".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut rx = handle.tx.subscribe();
    broadcast_persist_event(
        &store,
        &handle,
        "bp-fail",
        SessionEvent::Error("whatever".into()),
    )
    .await;

    let evt = rx.try_recv().expect("live frame survives a store outage");
    assert_eq!(evt.kind, "error");
    assert_eq!(
        store.events_after("bp-fail", 0).await.unwrap().len(),
        0,
        "the failed append must not have persisted anything"
    );
}
