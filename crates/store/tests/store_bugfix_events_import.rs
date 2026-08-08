//! Regression tests for two medium-severity store bugs:
//!
//! - `append_events_rejects_mixed_session_ids`: `append_events`/`append_many`
//!   used to capture `session_id = events[0].session_id` for the seq backfill
//!   while inserting each row under its own `ev.session_id`. A mixed batch
//!   returned seqs computed only for `events[0]`'s session — silently
//!   misaligned with no error. It must now reject a mixed batch up front and
//!   write no rows.
//! - `import_jsonl_failure_rolls_back_empty_session`: a partial JSONL import
//!   (`create_session` ok but `append_messages` failed) used to leave an empty
//!   session row committed, tripping the idempotency guard in
//!   `import_jsonl_dir` and blocking re-import forever. The stub row must now
//!   be rolled back so a retry can succeed.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use opencoder_core::Message;
use opencoder_store::{
    Delivery, EventKind, ImportReport, LibsqlStore, SessionEventRecord, SessionFilter,
    SessionInput, SessionListItem, SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};

fn make_session_meta(id: &str) -> SessionMeta {
    let now = 1_700_000_000i64;
    SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: None,
        model: None,
        workdir_hash: None,
        created_at: now,
        updated_at: now,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
    }
}

fn event(session_id: &str, n: i64, ts: i64) -> SessionEventRecord {
    SessionEventRecord {
        session_id: session_id.into(),
        kind: EventKind::TextDelta,
        payload: serde_json::json!({"n": n}),
        ts,
        seq: None,
        sse_kind: None,
    }
}

#[tokio::test]
async fn append_events_rejects_mixed_session_ids() {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("ev.db")).await.unwrap();
    // Both sessions must exist for the session_events FK constraint.
    store.create_session(&make_session_meta("sess-a")).await.unwrap();
    store.create_session(&make_session_meta("sess-b")).await.unwrap();

    let mixed = vec![event("sess-a", 0, 0), event("sess-b", 1, 1)];
    let err = store.append_events(&mixed).await;
    assert!(
        err.is_err(),
        "a mixed-session_id batch must be rejected, not silently misaligned"
    );
    let msg = format!("{}", err.unwrap_err());
    assert!(
        msg.contains("session_id"),
        "error should explain the session_id requirement, got: {msg}"
    );

    // No rows may be written for the rejected batch (validation bails pre-insert).
    assert_eq!(store.events_after("sess-a", 0).await.unwrap().len(), 0);
    assert_eq!(store.events_after("sess-b", 0).await.unwrap().len(), 0);

    // A homogeneous batch still works and assigns contiguous, monotonic seqs.
    let seqs = store
        .append_events(&[event("sess-a", 0, 0), event("sess-a", 1, 1)])
        .await
        .unwrap();
    assert_eq!(seqs.len(), 2);
    assert!(seqs[1] > seqs[0], "seqs must be monotonic within a batch");
}

/// Wraps a real `LibsqlStore` but makes `append_messages` fail, so we can
/// exercise the "session row created, then message append fails" path.
/// `delete_calls` counts rollbacks so we can assert the stub was cleaned up.
struct FailingAppendStore {
    inner: Arc<LibsqlStore>,
    delete_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Store for FailingAppendStore {
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
    async fn update_session(&self, id: &str, p: &SessionPatch) -> Result<()> {
        self.inner.update_session(id, p).await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        self.delete_calls.fetch_add(1, Ordering::Relaxed);
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, k: &str) -> Result<u64> {
        self.inner.clear_other_sessions(k).await
    }
    async fn append_message(&self, sid: &str, m: &Message) -> Result<i64> {
        self.inner.append_message(sid, m).await
    }
    async fn append_messages(&self, _sid: &str, _m: &[Message]) -> Result<Vec<i64>> {
        // Simulate a mid-import failure (disk error / constraint violation).
        anyhow::bail!("simulated append_messages failure")
    }
    async fn load_messages(&self, sid: &str) -> Result<Vec<Message>> {
        self.inner.load_messages(sid).await
    }
    async fn last_message_seq(&self, sid: &str) -> Result<i64> {
        self.inner.last_message_seq(sid).await
    }
    async fn admit_input(&self, i: &SessionInput) -> Result<i64> {
        self.inner.admit_input(i).await
    }
    async fn pending_inputs(&self, sid: &str, d: Delivery) -> Result<Vec<SessionInput>> {
        self.inner.pending_inputs(sid, d).await
    }
    async fn promote_inputs(&self, sid: &str, s: i64, d: Delivery) -> Result<Vec<i64>> {
        self.inner.promote_inputs(sid, s, d).await
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

#[tokio::test]
async fn import_jsonl_failure_rolls_back_empty_session() {
    let dir = tempfile::tempdir().unwrap();
    let jsonl_dir = dir.path().join("sessions");
    tokio::fs::create_dir_all(&jsonl_dir).await.unwrap();

    // Write a session jsonl with one user message.
    let m = Message::user("msg-1", "hello import");
    let path = jsonl_dir.join("fail-session.jsonl");
    let mut text = serde_json::to_string(&m).unwrap();
    text.push('\n');
    tokio::fs::write(&path, text).await.unwrap();

    let inner = Arc::new(LibsqlStore::open(dir.path().join("fail.db")).await.unwrap());
    let delete_calls = Arc::new(AtomicUsize::new(0));
    let failing = FailingAppendStore {
        inner: inner.clone(),
        delete_calls: delete_calls.clone(),
    };

    // First import: session row is created, append_messages fails, the stub
    // must be rolled back so it doesn't block a retry.
    let report = opencoder_store::import::import_jsonl_dir(&failing, &jsonl_dir)
        .await
        .unwrap();
    assert_eq!(report.sessions, 0, "no session fully imported");
    assert_eq!(report.skipped, 1, "the failed file is counted as skipped");

    // The empty session row MUST be gone — this is the regression.
    assert!(
        inner.get_session("fail-session").await.unwrap().is_none(),
        "empty stub session row must be rolled back, not left behind"
    );
    assert_eq!(
        delete_calls.load(Ordering::Relaxed),
        1,
        "delete_session must be invoked exactly once to roll back the stub"
    );

    // Re-import against the REAL (working) store now succeeds: the rollback
    // made the idempotency guard a no-op instead of a permanent block.
    let report2 = opencoder_store::import::import_jsonl_dir(&*inner, &jsonl_dir)
        .await
        .unwrap();
    assert_eq!(report2.sessions, 1, "retry must import the session cleanly");
    assert_eq!(report2.messages, 1);
    assert_eq!(inner.load_messages("fail-session").await.unwrap().len(), 1);
}
