//! In-process fan-out hub for DAG-run event SSE streams.
//!
//! Mirrors [`crate::nodes_state::NodeHub`] but keyed by DAG run id and
//! carrying [`DagEventView`] frames: a run's browser stream
//! `GET /api/dag/runs/:rid/events` has no drain runner either — events arrive
//! via node HTTP uploads (`POST /api/nodes/dag/runs/:rid/events`), which the
//! handlers persist FIRST and then publish here. Lifetime matches NodeHub:
//! lazily created on first subscribe, evicted by `cleanup` once the last
//! receiver drops (the SSE finalize guard calls it).
//!
//! The hub is reached through [`shared_dag_hub`] instead of an `AppState`
//! field: `AppState` is constructed as a struct literal in a dozen test
//! harnesses owned by other workstreams, so adding a field there would break
//! their builds. A process-wide singleton keyed by unique run ids (ULIDs) has
//! no cross-talk; `cleanup` keeps it bounded.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use opencoder_dag::DagEventView;
use tokio::sync::{broadcast, Mutex};

/// Replay covers persistence, so the channel only bridges short read stalls;
/// 256 frames of slack with bounded memory either way (same budget as NodeHub).
const CHANNEL_CAPACITY: usize = 256;

#[derive(Default)]
pub struct DagHub {
    map: Mutex<HashMap<String, broadcast::Sender<DagEventView>>>,
}

impl DagHub {
    pub fn new() -> Self {
        DagHub::default()
    }

    /// Get-or-create the broadcast channel for `run_id`. Subscribing FIRST
    /// (then querying persisted events) closes the subscribe/query race the
    /// same way the primary `/events` endpoint does; `created` tells the
    /// caller whether it brought the channel into existence.
    pub async fn subscribe(&self, run_id: &str) -> (broadcast::Receiver<DagEventView>, bool) {
        let mut map = self.map.lock().await;
        match map.get(run_id) {
            Some(tx) => (tx.subscribe(), false),
            None => {
                let (tx, _rx) = broadcast::channel::<DagEventView>(CHANNEL_CAPACITY);
                map.insert(run_id.to_string(), tx.clone());
                (tx.subscribe(), true)
            }
        }
    }

    /// Fan one frame out to the run's live subscribers. Nobody listening is a
    /// no-op — the frame is durable in the store regardless; backpressure
    /// from slow readers never blocks the uploading worker.
    pub async fn publish(&self, run_id: &str, evt: DagEventView) {
        let map = self.map.lock().await;
        if let Some(tx) = map.get(run_id) {
            let _ = tx.send(evt);
        }
    }

    /// Evict the channel when — and only when — nobody listens anymore.
    pub async fn cleanup(&self, run_id: &str) {
        let mut map = self.map.lock().await;
        let drop_it = map.get(run_id).is_some_and(|tx| tx.receiver_count() == 0);
        if drop_it {
            map.remove(run_id);
        }
    }
}

/// Process-wide hub singleton (see the module doc for why it is not an
/// `AppState` field). Pure accessor: always returns the same `Arc`.
pub fn shared_dag_hub() -> Arc<DagHub> {
    static HUB: OnceLock<Arc<DagHub>> = OnceLock::new();
    HUB.get_or_init(|| Arc::new(DagHub::new())).clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn view(seq: i64, kind: &str) -> DagEventView {
        DagEventView {
            seq,
            kind: kind.into(),
            step: None,
            payload: json!({}),
            at_ms: seq,
        }
    }

    /// Same behavioral contract as `nodes_state::NodeHub`: subscribe
    /// create/reuse, fan-out to every listener, strangers ignored, eviction
    /// only at zero receivers, singleton accessor stable.
    #[tokio::test]
    async fn subscribe_reuses_and_broadcast_reaches_all() {
        let hub = DagHub::new();
        let (mut r1, created1) = hub.subscribe("run-1").await;
        assert!(created1);
        let (mut r2, created2) = hub.subscribe("run-1").await;
        assert!(!created2);

        hub.publish("run-1", view(1, "run_started")).await;
        assert_eq!(r1.try_recv().unwrap().kind, "run_started");
        assert_eq!(r2.try_recv().unwrap().seq, 1);
    }

    #[tokio::test]
    async fn publish_to_unknown_run_is_a_noop() {
        let hub = DagHub::new();
        hub.publish("ghost", view(1, "run_finished")).await;
        let (mut rx, _) = hub.subscribe("run-1").await;
        hub.publish("run-1", view(2, "step_done")).await;
        assert_eq!(rx.recv().await.unwrap().kind, "step_done");
    }

    #[tokio::test]
    async fn cleanup_evicts_only_when_no_listeners_remain() {
        let hub = DagHub::new();
        let (rx_a, _) = hub.subscribe("run-1").await;
        let (rx_b, _) = hub.subscribe("run-1").await;

        hub.cleanup("run-1").await;
        let (_, kept) = hub.subscribe("run-1").await;
        assert!(!kept, "two live receivers keep the channel");

        drop(rx_a);
        hub.cleanup("run-1").await;
        let (_, kept) = hub.subscribe("run-1").await;
        assert!(!kept, "one live receiver still keeps the channel");

        drop(rx_b);
        // `kept` is the bool flag from subscribe, not the receiver — nothing
        // to drop; silence the copy-drop lint while keeping the shape.
        let _ = kept;
        hub.cleanup("run-1").await;
        let (_, recreated) = hub.subscribe("run-1").await;
        assert!(recreated, "evicted channel is rebuilt on demand");

        hub.cleanup("never-existed").await;
    }

    #[test]
    fn shared_hub_is_a_stable_singleton() {
        assert!(Arc::ptr_eq(&shared_dag_hub(), &shared_dag_hub()));
    }
}
