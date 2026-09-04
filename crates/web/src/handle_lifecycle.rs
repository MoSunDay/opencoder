//! Session-handle pinning and idle subscriber eviction.
//!
//! A lifecycle guard is only useful while every operation agrees on the same
//! handle instance. These helpers prevent SSE eviction from replacing an idle
//! handle between lookup and lock acquisition.
//!
//! 也收容「drain 之外」的广播助手（`broadcast_persist_event` /
//! `ensure_run_error_frame`）：它们与生命周期同属「无 drain 上下文时如何
//! 安全地把事件同时送到直播与持久层」的话题，放在这里让 `handle.rs`
//! 保持在文件预算内。

use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::sync::OwnedMutexGuard;

use anyhow::Result;
use opencoder_session::SessionEvent;
use opencoder_store::{SessionEventRecord, Store};
use tracing::warn;

use crate::handle::{sse_from_session_event, HandleMap, SessionHandle};

/// Return the current per-session handle with its lifecycle mutex held.
///
/// The identity check closes the lookup/lock race. Subscriber eviction also
/// takes this mutex before removal, so a verified handle stays mapped for the
/// full lifetime of the returned guard.
pub(crate) async fn lock_session_lifecycle(
    handles: &HandleMap,
    session_id: &str,
) -> (Arc<SessionHandle>, OwnedMutexGuard<()>) {
    loop {
        let handle = {
            let mut map = handles.lock().await;
            map.entry(session_id.to_string())
                .or_insert_with(SessionHandle::new)
                .clone()
        };
        let lifecycle = handle.lifecycle.clone().lock_owned().await;
        let still_current = handles
            .lock()
            .await
            .get(session_id)
            .is_some_and(|current| Arc::ptr_eq(current, &handle));
        if still_current {
            return (handle, lifecycle);
        }
    }
}

fn release_subscriber_slot(handle: &SessionHandle) -> usize {
    match handle
        .subscribers
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
            (value > 0).then_some(value.saturating_sub(1))
        }) {
        Ok(previous) => previous,
        Err(current) => current,
    }
}

async fn release_events_subscriber_async(handles: HandleMap, id: String) {
    let candidate = {
        let map = handles.lock().await;
        let Some(handle) = map.get(&id) else {
            return;
        };
        if release_subscriber_slot(handle) != 1 {
            return;
        }
        crate::handle_questions::abandon_all_waiting(handle);
        handle.clone()
    };
    // If an idle mutation currently owns the handle, wait for it to finish
    // before eviction. A queued operation revalidates identity in
    // `lock_session_lifecycle` and moves to the replacement handle.
    let _lifecycle = candidate.lifecycle.clone().lock_owned().await;
    let mut map = handles.lock().await;
    let still_current = map
        .get(&id)
        .is_some_and(|current| Arc::ptr_eq(current, &candidate));
    let evicted = still_current
        && candidate.subscribers.load(Ordering::SeqCst) == 0
        && !candidate.draining.load(Ordering::SeqCst);
    if evicted {
        map.remove(&id);
        drop(map);
        opencoder_session::mcp::cleanup(&id).await;
    }
}

/// Release an SSE subscriber and evict its idle handle when it was the last.
#[allow(unused_variables)]
pub(crate) fn release_events_subscriber(handles: HandleMap, id: String, created: bool) {
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(release_events_subscriber_async(handles, id));
    } else {
        std::thread::spawn(move || {
            if let Ok(runtime) = tokio::runtime::Runtime::new() {
                runtime.block_on(release_events_subscriber_async(handles, id));
            }
        });
    }
}

/// Broadcast + persist a session event from OUTSIDE a drain (switch endpoints,
/// resume-failure terminal frames). Live SSE subscribers receive the frame
/// immediately; the `session_events` append keeps replay (`?after=`) faithful
/// — the exact broadcast+sink contract `apply_drain_cmd` uses, for callers
/// that hold no `EventSink`.
///
/// Order is persist-then-broadcast: a subscriber arriving in the gap between
/// the two steps queries replay AFTER the append, so the row (seq > its
/// baseline) seeds the overlap fingerprint set and the live copy is swallowed
/// by `sse_dedup::forward_live` — delivered exactly once. Broadcast-first left
/// that subscriber with NEITHER copy: the live send had already flown by and
/// the replay query predated the row. 广播统一走 `broadcast_evt`（ring +
/// 直播），pre-subscribe gap 桥接同样覆盖这里发出的事件。
///
/// Store-write failures are warn-only: the live frame still goes out and a
/// missing replay row degrades gracefully.
pub(crate) async fn broadcast_persist_event(
    store: &Arc<dyn Store>,
    handle: &SessionHandle,
    session_id: &str,
    ev: SessionEvent,
) {
    let (sse, kind) = sse_from_session_event(session_id, &ev);
    let rec = SessionEventRecord {
        session_id: session_id.to_string(),
        kind,
        payload: ev.sse_data(),
        ts: opencoder_core::message::now_ms(),
        seq: None,
        sse_kind: Some(ev.sse_kind().to_string()),
    };
    if let Err(e) = store.append_event(&rec).await {
        warn!(session_id, error = %e, event = ev.sse_kind(), "broadcast event persist failed");
    }
    handle.broadcast_evt(sse);
}

/// P1 contract: a run that ends in `Err` owes the SSE consumer a terminal
/// `error` frame EVEN WHEN the runner surfaced none (control-command apply
/// failure, a store write failing mid-run, …) — otherwise the stream hangs
/// open forever with no terminal frame while `draining` resets. Runs that
/// already emitted their own `Error` are left alone: duplicating would
/// double-report the failure (and break the exactly-one-Error contract pinned
/// by `drain_no_restart_on_error`).
pub(crate) async fn ensure_run_error_frame(
    store: &Arc<dyn Store>,
    handle: &SessionHandle,
    session_id: &str,
    result: &Result<()>,
    run_emitted_error: bool,
) {
    if let Err(e) = result {
        if run_emitted_error {
            return;
        }
        broadcast_persist_event(
            store,
            handle,
            session_id,
            SessionEvent::Error(format!("{e:#}")),
        )
        .await;
    }
}
