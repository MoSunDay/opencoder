//! In-process fan-out hub for node-task SSE streams.
//!
//! A synthetic node session has NO `SessionHandle` in the shared [`HandleMap`]
//! (no drain runner owns it — events arrive via HTTP uploads, not a local LLM
//! loop). The [`NodeHub`] therefore keeps its own per-session broadcast
//! channel keyed by session id, so browsers can stream
//! `GET /api/nodes/tasks/:tid/events` while worker uploads are re-broadcast.
//!
//! Lifetime: the channel is created lazily on first subscribe and evicted by
//! `cleanup` once no receiver remains (called from the SSE finalize guard), so
//! the hub cannot grow without bound.
//!
//! [`HandleMap`]: crate::handle::HandleMap

use std::collections::HashMap;

use opencoder_core::SseEvt;
use tokio::sync::{broadcast, Mutex};

/// A node is considered `lost` when its last heartbeat is older than this.
/// Generous versus typical heartbeat intervals (2–5 s) so one dropped beat or
/// GC pause does not flap the UI to lost.
pub const STALE_AFTER_MS: i64 = 20_000;

#[derive(Default)]
pub struct NodeHub {
    map: Mutex<HashMap<String, broadcast::Sender<SseEvt>>>,
}

impl NodeHub {
    pub fn new() -> Self {
        NodeHub::default()
    }

    /// Get-or-create the broadcast channel for `sid`. Subscribing FIRST (then
    /// querying persisted events) closes the subscribe/query race, exactly like
    /// the primary `/events` endpoint. `created` tells the caller whether it
    /// brought the channel into existence.
    pub async fn subscribe(&self, sid: &str) -> (broadcast::Receiver<SseEvt>, bool) {
        let mut map = self.map.lock().await;
        match map.get(sid) {
            Some(tx) => (tx.subscribe(), false),
            None => {
                // 256 events of slack: replay covers persistence, so this only
                // bridges short read stalls; bounded memory either way.
                let (tx, _rx) = broadcast::channel::<SseEvt>(256);
                map.insert(sid.to_string(), tx.clone());
                (tx.subscribe(), true)
            }
        }
    }

    /// Fan an uploaded event out to live SSE subscribers of `sid`.
    /// No subscribers (or unknown sid): silently ignored — the event is
    /// durable in the store regardless; backpressure from slow readers never
    /// blocks the uploading worker's HTTP request.
    pub async fn broadcast(&self, sid: &str, evt: SseEvt) {
        let map = self.map.lock().await;
        if let Some(tx) = map.get(sid) {
            let _ = tx.send(evt);
        }
    }

    /// Evict the channel for `sid` if and only if nobody listens anymore.
    /// Called when an SSE stream ends; if another subscriber raced in first,
    /// the channel stays.
    pub async fn cleanup(&self, sid: &str) {
        let mut map = self.map.lock().await;
        let drop_it = map.get(sid).is_some_and(|tx| tx.receiver_count() == 0);
        if drop_it {
            map.remove(sid);
        }
    }
}

/// Derived liveness/status shown in `GET /api/nodes`.
///
/// Staleness dominates (`> STALE_AFTER_MS` ⇒ `"lost"`); otherwise the stored
/// raw status passes through when it is one of the known values and collapses
/// to `"online"` for anything unexpected (legacy/NULL rows). Note the boundary:
/// exactly `STALE_AFTER_MS` old is still alive.
pub fn compute_status(last_seen_at_ms: i64, last_status_raw: &str, now_ms: i64) -> &'static str {
    if now_ms - last_seen_at_ms > STALE_AFTER_MS {
        return "lost";
    }
    match last_status_raw {
        "online" => "online",
        "idle" => "idle",
        "busy" => "busy",
        _ => "online",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn evt(kind: &str) -> SseEvt {
        SseEvt {
            kind: kind.into(),
            data: json!({}),
            ts: 1,
            seq: None,
        }
    }

    #[test]
    fn stale_boundary_is_exclusive_and_raw_status_normalizes() {
        // Exactly STALE_AFTER_MS old → still alive (not >).
        assert_eq!(
            compute_status(1_000, "idle", 1_000 + STALE_AFTER_MS),
            "idle"
        );
        assert_eq!(
            compute_status(1_000, "online", 1_000 + STALE_AFTER_MS),
            "online"
        );
        // One ms past the window → lost.
        assert_eq!(
            compute_status(1_000, "idle", 1_000 + STALE_AFTER_MS + 1),
            "lost"
        );
        // Staleness dominates even an in-progress `busy`.
        assert_eq!(
            compute_status(1_000, "busy", 1_000 + STALE_AFTER_MS + 5),
            "lost"
        );
        // Fresh raw values pass through; garbage normalizes to `online`.
        assert_eq!(compute_status(5_000, "", 6_000), "online");
        assert_eq!(compute_status(5_000, "went-for-a-walk", 6_000), "online");
        assert_eq!(compute_status(5_000, "busy", 6_000), "busy");
    }

    #[tokio::test]
    async fn subscribe_created_once_reused_after() {
        let hub = NodeHub::new();
        let (mut r1, created1) = hub.subscribe("s").await;
        assert!(created1, "first subscribe creates the channel");
        let (mut r2, created2) = hub.subscribe("s").await;
        assert!(!created2, "second subscribe reuses it");

        // One broadcast reaches both subscribers independently.
        hub.broadcast("s", evt("done")).await;
        assert_eq!(r1.try_recv().unwrap().kind, "done");
        assert_eq!(r2.try_recv().unwrap().kind, "done");
    }

    #[tokio::test]
    async fn broadcast_reaches_subscribers_and_ignores_strangers() {
        let hub = NodeHub::new();
        // No channel at all: must not panic.
        hub.broadcast("ghost", evt("done")).await;
        let (mut rx, _) = hub.subscribe("s").await;
        hub.broadcast("s", evt("done")).await;
        assert_eq!(rx.recv().await.unwrap().kind, "done");
    }

    /// Cleanup semantics: only a zero-receiver channel is evicted; a racing
    /// new subscriber resurrects/reuses safely.
    #[tokio::test]
    async fn cleanup_evicts_only_when_no_listeners_remain() {
        let hub = NodeHub::new();
        let (rx_a, _) = hub.subscribe("s").await;
        let (_rx_b, _) = hub.subscribe("s").await;

        // Two listeners alive: cleanup keeps the channel.
        hub.cleanup("s").await;
        let (_, still_same) = hub.subscribe("s").await;
        assert!(!still_same);

        // One listener dropped: still someone left → keep.
        let _unused_flag = still_same; // discard the bool flags silently
        drop(rx_a);
        hub.cleanup("s").await;
        let (rx_c, kept) = hub.subscribe("s").await;
        assert!(!kept);

        // Last listener gone: cleanup evicts; next subscribe re-creates.
        drop(_rx_b);
        drop(rx_c);
        hub.cleanup("s").await;
        let (_, recreated) = hub.subscribe("s").await;
        assert!(recreated, "evicted channel must be rebuilt on demand");

        // Cleanup of an unknown sid is a no-op.
        hub.cleanup("never-existed").await;
    }
}
