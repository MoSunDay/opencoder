//! Session-handle pinning and idle subscriber eviction.
//!
//! A lifecycle guard is only useful while every operation agrees on the same
//! handle instance. These helpers prevent SSE eviction from replacing an idle
//! handle between lookup and lock acquisition.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::sync::OwnedMutexGuard;

use crate::handle::{HandleMap, SessionHandle};

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
