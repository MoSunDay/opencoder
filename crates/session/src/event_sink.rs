//! Buffered event persistence for high-frequency session surfaces (TUI / web /
//! subagent child streams).
//!
//! A single background flusher drains an unbounded channel of
//! [`SessionEventRecord`]s, batching the high-frequency delta variants
//! (`TextDelta` / `ReasoningDelta`, both coarse-mapped to
//! [`EventKind::TextDelta`]) into one transactional [`Store::append_events`]
//! call. Every other event flushes any pending deltas first, then persists
//! itself — so structural events are never reordered or coalesced, and a turn
//! boundary (channel close) triggers a final flush.
//!
//! This keeps the persisted event stream lossless on normal termination (channel close triggers a final flush). On a store *write* failure the batch is logged and dropped (warn-only) rather than retried — so the lossless guarantee holds for the buffering path, not for underlying store errors.
//! while collapsing the write count from O(tokens) to O(turn). Live UI/SSE
//! delivery is unaffected: each surface still forwards every event to its own
//! channel the instant it arrives — only the *disk* path is buffered. On a
//! crash the only thing at risk is a few un-flushed token fragments of the
//! in-flight turn; the turn's authoritative text still lands via the
//! per-turn `messages` append.

use std::sync::Arc;

use opencoder_core::message::now_ms;
use opencoder_store::{EventKind, SessionEventRecord, Store};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::warn;

use crate::SessionEvent;

/// Flush a delta buffer once it holds this many records.
const DELTA_BATCH: usize = 512;
/// ... or once its estimated serialized size (bytes) reaches this. Balances
/// batch size against the crash window (un-flushed tail of the live turn).
const DELTA_BYTES: usize = 8 * 1024;

/// A clonable handle that enqueues session events into an ordered, unbounded
/// buffer drained by a single [`spawn_event_flusher`] task. Cheap to clone
/// (shares one channel) so a run closure and the surrounding one-off emitters
/// in the same turn share the exact same ordered persistence path.
#[derive(Clone)]
pub struct EventSink {
    tx: mpsc::UnboundedSender<SessionEventRecord>,
    session_id: String,
}

impl EventSink {
    /// Build a persistence record for `sev` and enqueue it. Never blocks (the
    /// channel is unbounded), so it is safe to call from a sync event callback.
    /// Returns `Err(rec)` only if the flusher has already exited (channel
    /// closed) — i.e. after the run ended; such tail events are dropped.
    pub fn push(
        &self,
        sev: &SessionEvent,
    ) -> Result<(), mpsc::error::SendError<SessionEventRecord>> {
        let rec = SessionEventRecord {
            session_id: self.session_id.clone(),
            kind: sev.coarse_kind(),
            payload: sev.sse_data(),
            ts: now_ms(),
            seq: None,
            sse_kind: Some(sev.sse_kind().to_string()),
        };
        self.tx.send(rec)
    }
}

/// Spawn the single ordered flusher for a session and return a clonable push
/// handle plus the flusher's join handle.
///
/// **The join handle MUST be awaited** after the producing run returns (and
/// after dropping every clone of the [`EventSink`]) to guarantee the final
/// flush — zero event loss on normal termination. When `store` is `None` the
/// flusher simply drains and discards (in-memory sessions).
pub fn spawn_event_flusher(
    store: Option<Arc<dyn Store>>,
    session_id: String,
) -> (EventSink, JoinHandle<()>) {
    let (tx, rx) = mpsc::unbounded_channel::<SessionEventRecord>();
    let sink = EventSink {
        tx,
        session_id: session_id.clone(),
    };
    let handle = tokio::spawn(run_flusher(store, rx));
    (sink, handle)
}

/// Shared batching drain. Buffers delta records and flushes them in a single
/// transactional [`Store::append_events`]; any non-delta record flushes pending
/// deltas first (preserving coarse order) then persists itself. On channel
/// close (producer dropped) performs a final flush.
///
/// Exposed `pub` so the subagent / replay child-event flushers reuse the
/// identical batching + no-loss semantics — they build their own records (the
/// child payload is the whole event, not `sse_data`) but share this drain.
pub async fn run_flusher(
    store: Option<Arc<dyn Store>>,
    mut rx: mpsc::UnboundedReceiver<SessionEventRecord>,
) {
    let Some(store) = store else {
        // No durable store: drain to completion so the buffer never pins memory.
        while rx.recv().await.is_some() {}
        return;
    };
    let mut buf: Vec<SessionEventRecord> = Vec::new();
    let mut buf_bytes: usize = 0;
    while let Some(rec) = rx.recv().await {
        if rec.kind == EventKind::TextDelta {
            buf_bytes += approx_size(&rec);
            buf.push(rec);
            if buf.len() >= DELTA_BATCH || buf_bytes >= DELTA_BYTES {
                flush(&store, &buf).await;
                buf.clear();
                buf_bytes = 0;
            }
        } else {
            // Flush pending deltas first, then this structural event, so the
            // coarse ordering (deltas precede the event they led into) is exact.
            if !buf.is_empty() {
                flush(&store, &buf).await;
                buf.clear();
                buf_bytes = 0;
            }
            flush(&store, std::slice::from_ref(&rec)).await;
        }
    }
    // Channel closed (every producer dropped at turn/session end): final flush.
    if !buf.is_empty() {
        flush(&store, &buf).await;
    }
}

/// Approximate serialized footprint of a record, for the byte-threshold flush.
/// Uses the delta text length (the dominant cost) plus a small fixed overhead.
fn approx_size(rec: &SessionEventRecord) -> usize {
    let text = rec
        .payload
        .get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.len())
        .unwrap_or(0);
    text + 48
}

async fn flush(store: &Arc<dyn Store>, batch: &[SessionEventRecord]) {
    if batch.is_empty() {
        return;
    }
    if let Err(e) = store.append_events(batch).await {
        warn!(error = %e, count = batch.len(), "event batch flush failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::Message;
    use opencoder_store::{Delivery, LibsqlStore, SessionMeta};
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn fresh() -> (tempfile::TempDir, Arc<LibsqlStore>) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = Arc::new(LibsqlStore::open(dir.path().join("test.db")).await.unwrap());
        (dir, store)
    }

    async fn make_session(store: &Arc<LibsqlStore>, id: &str) {
        let meta = SessionMeta {
            id: id.into(),
            created_at: 1,
            updated_at: 1,
            ..Default::default()
        };
        store.create_session(&meta).await.unwrap();
    }

    /// Wraps a real store, counting `append_events` calls so we can assert the
    /// write count is O(turn) under a token storm while losing zero events.
    struct CountingStore {
        inner: Arc<LibsqlStore>,
        events_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Store for CountingStore {
        fn backend_name(&self) -> &'static str {
            self.inner.backend_name()
        }
        async fn create_session(&self, m: &SessionMeta) -> anyhow::Result<()> {
            self.inner.create_session(m).await
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
        async fn clear_other_sessions(&self, k: &str) -> anyhow::Result<u64> {
            self.inner.clear_other_sessions(k).await
        }
        async fn append_message(&self, sid: &str, m: &Message) -> anyhow::Result<i64> {
            self.inner.append_message(sid, m).await
        }
        async fn append_messages(&self, sid: &str, m: &[Message]) -> anyhow::Result<Vec<i64>> {
            self.inner.append_messages(sid, m).await
        }
        async fn load_messages(&self, sid: &str) -> anyhow::Result<Vec<Message>> {
            self.inner.load_messages(sid).await
        }
        async fn last_message_seq(&self, sid: &str) -> anyhow::Result<i64> {
            self.inner.last_message_seq(sid).await
        }
        async fn admit_input(&self, i: &opencoder_store::SessionInput) -> anyhow::Result<i64> {
            self.inner.admit_input(i).await
        }
        async fn pending_inputs(
            &self,
            sid: &str,
            d: Delivery,
        ) -> anyhow::Result<Vec<opencoder_store::SessionInput>> {
            self.inner.pending_inputs(sid, d).await
        }
        async fn promote_inputs(&self, sid: &str, s: i64, d: Delivery) -> anyhow::Result<Vec<i64>> {
            self.inner.promote_inputs(sid, s, d).await
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
        async fn delete_input(&self, id: i64) -> anyhow::Result<()> {
            self.inner.delete_input(id).await
        }
        async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> anyhow::Result<()> {
            self.inner.swap_input_order(sid, a, b).await
        }
        async fn append_events(&self, evs: &[SessionEventRecord]) -> anyhow::Result<Vec<i64>> {
            self.events_calls.fetch_add(1, Ordering::Relaxed);
            self.inner.append_events(evs).await
        }
        async fn events_after(&self, sid: &str, s: i64) -> anyhow::Result<Vec<SessionEventRecord>> {
            self.inner.events_after(sid, s).await
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
            res: &str,
            ok: bool,
        ) -> anyhow::Result<()> {
            self.inner.complete_subagent_task(id, res, ok).await
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

    // N text deltas + 1 structural event must persist every delta (no loss) while
    // calling append_events far fewer than N times (O(turn) writes).
    #[tokio::test]
    async fn deltas_persisted_losslessly_with_oturn_writes() {
        let (_dir, inner) = fresh().await;
        let store: Arc<CountingStore> = Arc::new(CountingStore {
            inner,
            events_calls: Arc::new(AtomicUsize::new(0)),
        });
        make_session(&store.inner, "s").await;

        let dyn_store: Arc<dyn Store> = store.clone();
        let (sink, flusher) = spawn_event_flusher(Some(dyn_store), "s".into());
        let n = 2000;
        for _ in 0..n {
            let _ = sink.push(&SessionEvent::TextDelta("x".into()));
        }
        // a structural (non-delta) event forces a flush of pending deltas first
        let _ = sink.push(&SessionEvent::Done);
        drop(sink);
        let _ = flusher.await;

        let all = store.inner.events_after("s", 0).await.unwrap();
        let text_deltas = all
            .iter()
            .filter(|r| r.sse_kind.as_deref() == Some("text_delta"))
            .count();
        assert_eq!(
            text_deltas, n,
            "every TextDelta must be persisted (no loss)"
        );
        let calls = store.events_calls.load(Ordering::Relaxed);
        assert!(
            calls < n / 10,
            "append_events called {calls} times for {n} deltas; expected O(turn) (<{})",
            n / 10
        );
        // the structural Done event survived too
        assert!(all.iter().any(|r| r.sse_kind.as_deref() == Some("done")));
    }

    // Non-delta events are never coalesced or reordered relative to deltas.
    #[tokio::test]
    async fn structural_events_interleave_in_order() {
        let (_dir, store) = fresh().await;
        make_session(&store, "s").await;

        let (sink, flusher) =
            spawn_event_flusher(Some(store.clone() as Arc<dyn Store>), "s".into());
        let _ = sink.push(&SessionEvent::TextDelta("a".into()));
        let _ = sink.push(&SessionEvent::TextDelta("b".into()));
        let _ = sink.push(&SessionEvent::Status("mid".into()));
        let _ = sink.push(&SessionEvent::TextDelta("c".into()));
        let _ = sink.push(&SessionEvent::Done);
        drop(sink);
        let _ = flusher.await;

        let all = store.events_after("s", 0).await.unwrap();
        let kinds: Vec<&str> = all
            .iter()
            .map(|r| r.sse_kind.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(
            kinds,
            vec!["text_delta", "text_delta", "status", "text_delta", "done"]
        );
        assert_eq!(all.len(), 5);
        // seqs strictly increasing (emission order preserved)
        let seqs: Vec<i64> = all.iter().map(|r| r.seq.unwrap()).collect();
        let mut sorted = seqs.clone();
        sorted.sort_unstable();
        assert_eq!(seqs, sorted);
    }

    // A store-less (in-memory) session must still drain cleanly without panicking.
    #[tokio::test]
    async fn no_store_drains_without_panic() {
        let (sink, flusher) = spawn_event_flusher(None, "mem".into());
        for _ in 0..100 {
            let _ = sink.push(&SessionEvent::TextDelta("z".into()));
        }
        drop(sink);
        let _ = flusher.await;
    }
}
