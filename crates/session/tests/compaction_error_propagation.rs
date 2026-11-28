//! Compaction error-propagation: when the durable store rejects the
//! `update_session` write that persists compaction metadata, `compact()` must
//! propagate the error (`Err`) rather than silently swallowing it (the
//! pre-hardening `let _ =` behaviour). This is the error-path companion to the
//! happy-path integration coverage in `compaction_and_model.rs`.
//!
//! Every write failure -- unreadable storage (`SQLITE_CORRUPT`, which can be
//! transient) or any other kind -- gets a bounded in-place retry of the write,
//! never a rerun of the summarize LLM call, and a persist failure is tagged so
//! the runner stops retrying.
//!
//! Both exits also pin the status-line contract: the `compacting conversation…`
//! status raised before the write is cleared when `compact` returns `Err` (the
//! status guard in `compact`), and a successful -- possibly retried -- run
//! still ends with the status cleared by the pipeline itself.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use opencoder_core::{resolve_agent, Config, Message};
use opencoder_llm::{CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{
    compaction::{compact, is_persist_failure},
    SessionEvent, SessionState,
};
use opencoder_store::{
    Delivery, ImportReport, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
    SessionMeta, SessionPatch, Store, SubagentTaskRecord, TursoStore,
};

/// The production error shape for unreadable storage: the driver reports
/// `SQLITE_CORRUPT` (a page reading as all zeros right after a rolled-back
/// statement). The store adds no context of its own; the persistence context
/// is added exclusively by `persist_summary`, which the tests below pin.
fn unreadable() -> anyhow::Error {
    anyhow::Error::new(turso::Error::Corrupt("Invalid page type: 0".into()))
}

/// A write failure that is neither corruption nor a worker panic.
fn plain() -> anyhow::Error {
    anyhow::anyhow!("disk I/O error")
}

/// Wraps a real in-memory `TursoStore` but makes the first `failures`
/// `update_session` calls fail with `failure()`, simulating a store that
/// rejects compaction metadata persistence. Once the budget is spent the call
/// delegates to the inner store.
struct FailingUpdateStore {
    inner: Arc<TursoStore>,
    /// Leading `update_session` calls that fail (`usize::MAX`: never delegate).
    failures: AtomicUsize,
    /// Every `update_session` call observed, failed or delegated.
    calls: AtomicUsize,
    /// Fresh error for every failing call (`anyhow::Error` is not `Clone`).
    failure: fn() -> anyhow::Error,
}

#[async_trait]
impl Store for FailingUpdateStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create_session(&self, m: &SessionMeta) -> Result<()> {
        self.inner.create_session(m).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        self.inner.get_session(id).await
    }
    async fn list_sessions(&self, f: &SessionFilter) -> Result<Vec<SessionListItem>> {
        self.inner.list_sessions(f).await
    }
    async fn update_session(&self, id: &str, patch: &SessionPatch) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // `try_update` (the non-deprecated name of `fetch_update`): the
        // closure returns `None` at 0, i.e. the failure budget is spent.
        let still_failing = self
            .failures
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
        if still_failing.is_ok() {
            return Err((self.failure)());
        }
        self.inner.update_session(id, patch).await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, keep: &str) -> Result<u64> {
        self.inner.clear_other_sessions(keep).await
    }
    async fn append_message(&self, sid: &str, m: &Message) -> Result<i64> {
        self.inner.append_message(sid, m).await
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
    async fn delete_input(&self, id: i64) -> Result<()> {
        self.inner.delete_input(id).await
    }
    async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> Result<()> {
        self.inner.swap_input_order(sid, a, b).await
    }
    async fn append_events(&self, evs: &[SessionEventRecord]) -> Result<Vec<i64>> {
        self.inner.append_events(evs).await
    }
    async fn events_after(&self, sid: &str, after: i64) -> Result<Vec<SessionEventRecord>> {
        self.inner.events_after(sid, after).await
    }
    async fn last_event_seq(&self, sid: &str) -> Result<i64> {
        self.inner.last_event_seq(sid).await
    }
    async fn create_subagent_task(&self, r: &SubagentTaskRecord) -> Result<()> {
        self.inner.create_subagent_task(r).await
    }
    async fn complete_subagent_task(&self, id: &str, res: &str, ok: bool) -> Result<()> {
        self.inner.complete_subagent_task(id, res, ok).await
    }
    async fn list_subagent_tasks(&self, sid: &str) -> Result<Vec<SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(sid).await
    }
    async fn get_subagent_task(&self, id: &str) -> Result<Option<SubagentTaskRecord>> {
        self.inner.get_subagent_task(id).await
    }
    async fn cancel_subagent_task(&self, id: &str) -> Result<()> {
        self.inner.cancel_subagent_task(id).await
    }
    async fn import_messages(&self, sid: &str, msgs: &[Message]) -> Result<ImportReport> {
        self.inner.import_messages(sid, msgs).await
    }
}

/// Everything a compaction run against a failing store reports back.
struct Run {
    outcome: Result<Option<String>>,
    events: Vec<SessionEvent>,
    /// Summarizer LLM calls made.
    llm_calls: usize,
    /// `update_session` calls observed, failed or delegated.
    writes: usize,
}

/// Compact a two-turn session whose store rejects its leading `failures`
/// metadata writes with `failure`. The clock is paused so the bounded backoff
/// auto-advances instead of sleeping.
async fn compact_with_failing_writes(failures: usize, failure: fn() -> anyhow::Error) -> Run {
    tokio::time::pause();
    let mock = Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: "conversation summary".into(),
            tool_calls: Vec::<CompletedToolCall>::new(),
            usage: Some(Usage {
                input_tokens: 5,
                output_tokens: 3,
                total_tokens: 8,
                ..Default::default()
            }),
        }]),
    );
    let store = Arc::new(FailingUpdateStore {
        inner: Arc::new(TursoStore::open_memory().await.unwrap()),
        failures: AtomicUsize::new(failures),
        calls: AtomicUsize::new(0),
        failure,
    });

    let agent = resolve_agent("act").expect("act agent resolves");
    let mut s = SessionState::new(
        "compact-persist",
        agent,
        Config {
            model: "main/glm-5.2".into(),
            ..Config::default()
        },
        mock.clone(),
        std::env::temp_dir(),
    )
    .with_store(store.clone());

    // Two turns so compaction_split returns a real head/tail (head summarizable).
    s.messages.push(Message::user("u1", "first turn"));
    s.messages.push(Message::assistant("a1"));
    s.messages.push(Message::user("u2", "second turn"));
    s.messages.push(Message::assistant("a2"));

    let mut events: Vec<SessionEvent> = Vec::new();
    let outcome = compact(&mut s, &HashMap::new(), &mut |ev| events.push(ev)).await;
    Run {
        outcome,
        events,
        llm_calls: mock.call_count(),
        writes: store.calls.load(Ordering::SeqCst),
    }
}

fn status(run: &Run, needle: &str) -> bool {
    run.events
        .iter()
        .any(|ev| matches!(ev, SessionEvent::Status(m) if m.contains(needle)))
}

/// The most recent `Status` payload -- what the status line shows once the
/// run has finished.
fn last_status(run: &Run) -> Option<&str> {
    run.events.iter().rev().find_map(|ev| match ev {
        SessionEvent::Status(m) => Some(m.as_str()),
        _ => None,
    })
}

/// When the store rejects the `update_session` write that persists compaction
/// metadata, `compact()` must return `Err` whose outermost message is the
/// "persist compaction metadata" context tag -- not silently drop the failure.
/// The summary LLM call succeeds (MockChatClient) so we reach the store-write
/// line. The failure is permanent here (the write never recovers), so the error
/// must also carry the persistence marker that tells the runner not to rerun
/// the summarizer, and the write budget must be spent exactly.
#[tokio::test]
async fn compact_returns_err_when_store_rejects_metadata_persistence() {
    let run = compact_with_failing_writes(usize::MAX, unreadable).await;
    let error = run
        .outcome
        .as_ref()
        .expect_err("compact must return Err when the store rejects the metadata write");
    // The store error carries no context of its own: the tag is added by
    // `persist_summary` alone, so the outermost anyhow message is exactly it.
    assert_eq!(
        error.to_string(),
        "persist compaction metadata",
        "the outermost message must be the persistence context tag"
    );
    assert!(
        format!("{error:#}").contains("Invalid page type: 0"),
        "the store error must stay reachable in the chain, got: {error:#}"
    );
    assert!(
        is_persist_failure(error),
        "a persist failure must carry the marker that stops the runner's retry, got: {error:#}"
    );
    assert_eq!(run.writes, 3, "one attempt plus the two bounded retries");
    assert_eq!(
        run.llm_calls, 1,
        "the summarizer must run once even when every write fails"
    );
    assert!(
        status(&run, "compacting conversation\u{2026}"),
        "compaction must report a compacting status before the store write"
    );
    assert!(
        status(&run, "retrying 2/2"),
        "the last retry must be reported, got: {:?}",
        run.events
    );
    // The persist failure leaves through `?` after the compacting status was
    // raised: the status guard must clear it so the frontend's status line is
    // not left stuck on a compaction that already gave up.
    assert_eq!(
        last_status(&run),
        Some(""),
        "the compacting status must be cleared after the persist failure, got: {:?}",
        run.events
    );
}

/// A transient unreadable-storage failure on the metadata write is retried in
/// place (bounded backoff) and the summarizer LLM call is NOT rerun: the
/// summary already exists in memory, so only the write is repeated.
#[tokio::test]
async fn compact_retries_the_write_without_rerunning_the_summarizer() {
    let run = compact_with_failing_writes(1, unreadable).await;
    assert!(
        run.outcome.is_ok(),
        "compact must succeed once the retried write lands, got: {:?}",
        run.outcome
    );
    assert_eq!(
        run.llm_calls, 1,
        "the summarizer LLM call must run exactly once across the write retry"
    );
    assert_eq!(
        run.writes, 2,
        "the metadata write must be retried exactly once"
    );
    assert!(
        status(&run, "retrying 1/2"),
        "the write retry must be reported as a status event, got: {:?}",
        run.events
    );
    // Success keeps its original status behaviour: the pipeline itself clears
    // the status once the retried write lands (the guard adds nothing on Ok).
    assert_eq!(
        last_status(&run),
        Some(""),
        "a successful run must end with a cleared status"
    );
}

/// The write retry does not classify by error kind: a plain failure that is
/// neither corruption nor a worker panic gets the same bounded retries and the
/// same persist marker, so the runner never falls back to rerunning the
/// summarizer for a write that keeps failing.
#[tokio::test]
async fn compact_retries_every_write_failure_not_only_unreadable_storage() {
    let run = compact_with_failing_writes(usize::MAX, plain).await;
    let error = run
        .outcome
        .as_ref()
        .expect_err("compact must return Err when the write keeps failing");
    assert_eq!(error.to_string(), "persist compaction metadata");
    assert!(
        is_persist_failure(error),
        "a non-storage write failure must be tagged as a persist failure too, got: {error:#}"
    );
    assert_eq!(
        run.writes, 3,
        "every write failure is retried up to the budget"
    );
    assert_eq!(run.llm_calls, 1, "the summarizer must run exactly once");
}
