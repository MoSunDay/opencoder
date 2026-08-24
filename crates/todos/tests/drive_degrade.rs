//! `drive()` error-path hardening: the takeover probe (`persistence::load`)
//! must never mask the runtime error it diagnoses, nor skip the local
//! suspension commit when the store itself fails mid-probe.
//!
//! The bug this pins: `drive`'s `if let Err(error) = result` branch used to
//! propagate the probe's own failure with `?` — a store error during the
//! probe (a) replaced the ORIGINAL runtime error in the returned `Err` and
//! (b) skipped the whole suspend-to-store flow, leaving the workflow stuck
//! `Running` in persistence with no `runtime_error` event. The fix degrades
//! the probe failure to a `warn` + `None` so the original error keeps
//! propagating and the local suspension still lands.

use std::{path::Path, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{Config, Message};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{
    LibsqlStore, SessionEventRecord, SessionFilter, SessionInput, SessionListItem, SessionMeta,
    SessionPatch, Store, SubagentTaskRecord, TodoEventRecord, TodoItemRecord, TodoWorkflowRecord,
};
use opencoder_todos::{types::*, Runtime};
use tokio_util::sync::CancellationToken;

/// A delegating store whose `get_todo_workflow` FAILS once the workflow row
/// exists — exactly the probe-after-create window `drive`'s error path hits.
/// Every earlier call (the `run_new_with_id` existence check) sees no row and
/// passes through untouched, so the failure lands precisely on the takeover
/// probe inside the error path.
struct ProbeFailingStore {
    inner: Arc<LibsqlStore>,
}

#[async_trait]
impl Store for ProbeFailingStore {
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
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, k: &str) -> Result<u64> {
        self.inner.clear_other_sessions(k).await
    }
    async fn append_message(&self, sid: &str, m: &Message) -> Result<i64> {
        self.inner.append_message(sid, m).await
    }
    async fn append_messages(&self, sid: &str, m: &[Message]) -> Result<Vec<i64>> {
        self.inner.append_messages(sid, m).await
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
    async fn pending_inputs(
        &self,
        sid: &str,
        d: opencoder_store::Delivery,
    ) -> Result<Vec<SessionInput>> {
        self.inner.pending_inputs(sid, d).await
    }
    async fn promote_inputs(
        &self,
        sid: &str,
        up: i64,
        d: opencoder_store::Delivery,
    ) -> Result<Vec<i64>> {
        self.inner.promote_inputs(sid, up, d).await
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
    async fn append_events(&self, ev: &[SessionEventRecord]) -> Result<Vec<i64>> {
        self.inner.append_events(ev).await
    }
    async fn events_after(&self, sid: &str, s: i64) -> Result<Vec<SessionEventRecord>> {
        self.inner.events_after(sid, s).await
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
    async fn list_subagent_tasks(&self, pid: &str) -> Result<Vec<SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(pid).await
    }
    async fn get_subagent_task(&self, id: &str) -> Result<Option<SubagentTaskRecord>> {
        self.inner.get_subagent_task(id).await
    }
    async fn cancel_subagent_task(&self, id: &str) -> Result<()> {
        self.inner.cancel_subagent_task(id).await
    }
    async fn create_todo_workflow(
        &self,
        w: &TodoWorkflowRecord,
        items: &[TodoItemRecord],
        ev: &TodoEventRecord,
    ) -> Result<i64> {
        self.inner.create_todo_workflow(w, items, ev).await
    }
    async fn get_todo_workflow(&self, id: &str) -> Result<Option<TodoWorkflowRecord>> {
        match self.inner.get_todo_workflow(id).await {
            // No row yet (the run_new existence check) — pass through.
            Ok(None) => Ok(None),
            // Row exists (the takeover probe after drive_inner failed) —
            // explode, simulating a transient store failure at the worst
            // possible moment.
            Ok(Some(_)) => anyhow::bail!("store exploded during takeover probe"),
            Err(e) => Err(e),
        }
    }
    async fn commit_todo_transition(
        &self,
        w: &TodoWorkflowRecord,
        items: &[TodoItemRecord],
        ev: &TodoEventRecord,
    ) -> Result<i64> {
        self.inner.commit_todo_transition(w, items, ev).await
    }
    async fn todo_events_after(&self, id: &str, after: i64) -> Result<Vec<TodoEventRecord>> {
        self.inner.todo_events_after(id, after).await
    }
}

fn spec() -> WorkflowSpec {
    WorkflowSpec {
        schema_version: 1,
        id: "wf-test".into(),
        name: "test".into(),
        objective: "finish one item".into(),
        constraints: Vec::new(),
        todos: vec![TodoSpec {
            id: "step-1".into(),
            title: "step".into(),
            requirement_background: "required by test".into(),
            instructions: "return the candidate".into(),
            depends_on: Vec::new(),
            agent: "act".into(),
            max_attempts: 2,
            allowed_tools: vec![],
            acceptance: AcceptanceSpec {
                criteria: "candidate exists".into(),
                required_tool_calls: Vec::new(),
            },
            metadata: serde_json::Value::Null,
        }],
        metadata: serde_json::Value::Null,
    }
}

fn make_runtime(store: &Arc<dyn Store>, client: Arc<dyn ChatStream>, dir: &Path) -> Runtime {
    Runtime {
        store: store.clone(),
        client,
        config: Config::default(),
        workdir: dir.to_path_buf(),
        debug_root: None,
        cancel: CancellationToken::new(),
    }
}

/// drive_inner fails (mock exhausted on the first parent dispatch call) and
/// the takeover probe's own load fails: the ORIGINAL error must propagate
/// (not the store error) and the workflow must still be suspended + recorded
/// as `runtime_error` in the real store.
#[tokio::test]
async fn probe_failure_degrades_and_original_error_survives() {
    let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let store: Arc<dyn Store> = Arc::new(ProbeFailingStore {
        inner: inner.clone(),
    });
    // Zero scripts: the very first parent LLM call fails, so drive_inner
    // errors before any child session or interrupt poll exists.
    let mock = Arc::new(MockChatClient::new());
    let temp = tempfile::tempdir().unwrap();
    let runtime = make_runtime(&store, mock, temp.path());

    let err = runtime
        .run_new_with_id(spec(), "run-probe-fail".into())
        .await
        .expect_err("drive_inner must fail");

    let rendered = format!("{err:#}");
    assert!(
        rendered.contains("mock exhausted"),
        "the ORIGINAL runtime error must propagate, got: {rendered}"
    );
    assert!(
        !rendered.contains("store exploded"),
        "the probe's own store failure must not replace the runtime error: {rendered}"
    );

    // The local suspension still landed in the real store.
    let (_, state) =
        opencoder_todos::persistence::load(&(inner.clone() as Arc<dyn Store>), "run-probe-fail")
            .await
            .unwrap()
            .expect("workflow must be persisted");
    assert_eq!(state.status, WorkflowStatus::Suspended);
    let events = inner.todo_events_after("run-probe-fail", 0).await.unwrap();
    assert_eq!(
        events.last().map(|e| e.kind.as_str()),
        Some("runtime_error"),
        "suspension must be recorded even when the probe fails"
    );
}
