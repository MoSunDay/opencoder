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

