//! In-memory control-plane hub for the P3 message relay (server side).
//!
//! Conversation content never lives on the server: the fleet console asks the
//! WORKER for a resume-shaped slice, relayed through two HTTP hops. This hub
//! is the rendezvous point between those hops:
//!
//! * `push` / `pop` — a per-node FIFO of [`ControlTask`] the worker picks up
//!   opportunistically (claim reply + heartbeat) because a busy worker never
//!   polls claim.
//! * `register` / `resolve` / `abandon` — one-shot pending-result registry
//!   keyed by `control_id`: the browser-facing handler registers a oneshot
//!   receiver, pushes the task, and awaits; the worker's result upload
//!   resolves the waiter. NOTHING here touches the store — payloads are
//!   relayed, never persisted.
//!
//! Lifetime: entries disappear on pop (queue) and on resolve/abandon
//! (registry); `purge_node` cleans up when a node is deleted.

use std::collections::{HashMap, VecDeque};

use opencoder_core::node_protocol::{ControlTask, FetchMessagesResult};
use tokio::sync::{oneshot, Mutex};

/// Upper bound on control tasks handed out per heartbeat. A worker that goes
/// silent mid-batch would stall the queue otherwise; the small cap keeps one
/// heartbeat's reply (and the worker's fan-out) bounded.
pub const HEARTBEAT_CONTROL_BATCH: usize = 4;

/// Default browser-facing await window for a control round-trip. Generous
/// enough for a worker to answer on its next heartbeat (2-5 s interval) plus
/// local read time; the HTTP handler accepts a client `timeout_ms` hint that
/// is capped by [`MAX_RELAY_TIMEOUT_MS`].
pub const DEFAULT_RELAY_TIMEOUT_MS: u64 = 15_000;
/// Hard cap for the client-supplied `timeout_ms` hint (keeps a stale HTTP
/// request from pinning a handler task indefinitely).
pub const MAX_RELAY_TIMEOUT_MS: u64 = 15_000;

#[derive(Default)]
pub struct ControlHub {
    /// node_id -> FIFO of undelivered control tasks.
    queues: Mutex<HashMap<String, VecDeque<ControlTask>>>,
    /// control_id -> pending browser waiter (one-shot).
    pending: Mutex<HashMap<String, oneshot::Sender<FetchMessagesResult>>>,
}

impl ControlHub {
    pub fn new() -> Self {
        ControlHub::default()
    }

    /// Queue one control task for `node_id` (FIFO, delivery at claim/heartbeat).
    pub async fn push(&self, node_id: &str, task: ControlTask) {
        let mut q = self.queues.lock().await;
        q.entry(node_id.to_string()).or_default().push_back(task);
    }

    /// Dequeue the OLDEST queued control task for `node_id`, if any.
    pub async fn pop(&self, node_id: &str) -> Option<ControlTask> {
        self.pop_many(node_id, 1).await.into_iter().next()
    }

    /// Dequeue up to `max` oldest control tasks for `node_id` (heartbeat
    /// batch). Empty result removes the node's entry so an idle fleet cannot
    /// grow the map.
    pub async fn pop_many(&self, node_id: &str, max: usize) -> Vec<ControlTask> {
        if max == 0 {
            return Vec::new();
        }
        let mut q = self.queues.lock().await;
        let Some(entry) = q.get_mut(node_id) else {
            return Vec::new();
        };
        let out: Vec<ControlTask> = entry.drain(..max.min(entry.len())).collect();
        if entry.is_empty() {
            q.remove(node_id);
        }
        out
    }

    /// Register a pending result waiter for `control_id`; the returned
    /// receiver is what the browser-facing handler awaits.
    pub async fn register(&self, control_id: &str) -> oneshot::Receiver<FetchMessagesResult> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(control_id.to_string(), tx);
        rx
    }

    /// Deliver a worker's result to the waiting browser request. `true` when
    /// a waiter existed and was woken; `false` for unknown/stale ids (the
    /// HTTP layer still answers 200 so a lagging worker is never punished).
    pub async fn resolve(&self, control_id: &str, result: FetchMessagesResult) -> bool {
        let waiter = self.pending.lock().await.remove(control_id);
        match waiter {
            Some(tx) => tx.send(result).is_ok(),
            None => false,
        }
    }

    /// Drop a waiter without a result (browser-side timeout path); a late
    /// worker upload then resolves against a removed id and reports `false`.
    pub async fn abandon(&self, control_id: &str) -> bool {
        self.pending.lock().await.remove(control_id).is_some()
    }

    /// Number of waiters still pending (observability / tests).
    pub async fn pending_count(&self) -> usize {
        self.pending.lock().await.len()
    }

    /// Forget a node's undelivered queue (node deleted). Pending waiters are
    /// NOT resolved here: their own timeout handles the dead node.
    pub async fn purge_node(&self, node_id: &str) -> usize {
        let mut q = self.queues.lock().await;
        q.remove(node_id).map(|v| v.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(control_id: &str, session_id: &str) -> ControlTask {
        ControlTask {
            control_id: control_id.into(),
            kind: "fetch_messages".into(),
            session_id: session_id.into(),
        }
    }

    fn result(control_id: &str, ok: bool) -> FetchMessagesResult {
        FetchMessagesResult {
            control_id: control_id.into(),
            session_id: "s".into(),
            ok,
            error: None,
            summary: None,
            summary_seq: None,
            messages: vec![],
        }
    }

    /// Per-node FIFO: oldest in, first out; nodes are isolated.
    #[tokio::test]
    async fn pop_is_fifo_per_node() {
        let hub = ControlHub::new();
        hub.push("n1", task("c1", "s")).await;
        hub.push("n1", task("c2", "s")).await;
        hub.push("n2", task("c3", "s")).await;

        assert_eq!(hub.pop("n1").await.unwrap().control_id, "c1");
        assert_eq!(hub.pop("n1").await.unwrap().control_id, "c2");
        assert!(hub.pop("n1").await.is_none(), "queue drains empty");
        // Other node's queue untouched by n1's drains.
        assert_eq!(hub.pop("n2").await.unwrap().control_id, "c3");
    }

    /// `pop_many` takes the oldest batch at most `max` and clears the entry.
    #[tokio::test]
    async fn pop_many_batches_in_fifo_order() {
        let hub = ControlHub::new();
        for id in ["c1", "c2", "c3"] {
            hub.push("n", task(id, "s")).await;
        }
        let batch = hub.pop_many("n", 2).await;
        let ids: Vec<String> = batch.into_iter().map(|t| t.control_id).collect();
        assert_eq!(ids, ["c1", "c2"]);
        let rest = hub.pop_many("n", 8).await;
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].control_id, "c3");
        assert!(hub.pop_many("n", 4).await.is_empty());
        assert_eq!(hub.pop_many("n", 0).await.len(), 0, "max=0 is a no-op");
    }

    /// resolve wakes the registered waiter with the exact payload.
    #[tokio::test]
    async fn resolve_wakes_waiter_with_payload() {
        let hub = ControlHub::new();
        let rx = hub.register("c1").await;
        assert_eq!(hub.pending_count().await, 1);

        assert!(hub.resolve("c1", result("c1", true)).await);
        let got = rx.await.expect("waiter must be woken");
        assert!(got.ok);
        assert_eq!(got.control_id, "c1");
        assert_eq!(hub.pending_count().await, 0, "waiter consumed");
    }

    /// resolve for an unknown/stale id is `false` and answers 200 upstream.
    #[tokio::test]
    async fn resolve_unknown_id_is_false() {
        let hub = ControlHub::new();
        assert!(!hub.resolve("ghost", result("ghost", true)).await);
        // After abandon (browser timeout), a late upload also reports false.
        let _rx = hub.register("late").await;
        assert!(hub.abandon("late").await);
        assert!(!hub.abandon("late").await, "second abandon sees nothing");
        assert!(!hub.resolve("late", result("late", true)).await);
    }

    /// A dropped receiver resolves false instead of panicking the uploader.
    #[tokio::test]
    async fn resolve_after_receiver_dropped_is_false() {
        let hub = ControlHub::new();
        let rx = hub.register("c").await;
        drop(rx);
        assert!(!hub.resolve("c", result("c", true)).await);
    }

    /// purge_node drops only that node's undelivered queue.
    #[tokio::test]
    async fn purge_node_clears_its_queue_only() {
        let hub = ControlHub::new();
        hub.push("n1", task("c1", "s")).await;
        hub.push("n2", task("c2", "s")).await;
        assert_eq!(hub.purge_node("n1").await, 1);
        assert!(hub.pop("n1").await.is_none());
        assert_eq!(hub.pop("n2").await.unwrap().control_id, "c2");
        assert_eq!(hub.purge_node("never").await, 0);
    }
}
