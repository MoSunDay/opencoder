//! M5 regression: `interrupt` runs load→mutate→CAS-commit while the driving
//! process bumps the generation on every transition, so the commit can lose
//! the optimistic-concurrency race. A generation conflict must be retried
//! (reloading the freshest state, re-deriving the terminal check) instead of
//! failing the interrupt.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use opencoder_core::{Config, Message};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_store::{
    LibsqlStore, SessionEventRecord, SessionFilter, SessionInput, SessionListItem, SessionMeta,
    SessionPatch, Store, SubagentTaskRecord, TodoEventRecord, TodoItemRecord, TodoWorkflowRecord,
};
use opencoder_todos::{types::*, Runtime};
use tokio_util::sync::CancellationToken;

/// Delegating store whose `commit_todo_transition` fails exactly ONCE with
/// the store's real generation-conflict error (the tx rolls back, so no
/// event lands), but only for the interrupt commit — simulating the driving
/// process winning the CAS race on the first attempt.
struct ConflictOnceStore {
    inner: Arc<LibsqlStore>,
    fired: AtomicBool,
}

#[async_trait::async_trait]
impl Store for ConflictOnceStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create_session(&self, m: &SessionMeta) -> Result<(), anyhow::Error> {
        self.inner.create_session(m).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>, anyhow::Error> {
        self.inner.get_session(id).await
    }
    async fn list_sessions(
        &self,
        f: &SessionFilter,
    ) -> Result<Vec<SessionListItem>, anyhow::Error> {
        self.inner.list_sessions(f).await
    }
    async fn update_session(&self, id: &str, p: &SessionPatch) -> Result<(), anyhow::Error> {
        self.inner.update_session(id, p).await
    }
    async fn delete_session(&self, id: &str) -> Result<(), anyhow::Error> {
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, k: &str) -> Result<u64, anyhow::Error> {
        self.inner.clear_other_sessions(k).await
    }
    async fn append_message(&self, sid: &str, m: &Message) -> Result<i64, anyhow::Error> {
        self.inner.append_message(sid, m).await
    }
    async fn append_messages(&self, sid: &str, m: &[Message]) -> Result<Vec<i64>, anyhow::Error> {
        self.inner.append_messages(sid, m).await
    }
    async fn load_messages(&self, sid: &str) -> Result<Vec<Message>, anyhow::Error> {
        self.inner.load_messages(sid).await
    }
    async fn last_message_seq(&self, sid: &str) -> Result<i64, anyhow::Error> {
        self.inner.last_message_seq(sid).await
    }
    async fn admit_input(&self, input: &SessionInput) -> Result<i64, anyhow::Error> {
        self.inner.admit_input(input).await
    }
    async fn pending_inputs(
        &self,
        sid: &str,
        d: opencoder_store::Delivery,
    ) -> Result<Vec<SessionInput>, anyhow::Error> {
        self.inner.pending_inputs(sid, d).await
    }
    async fn promote_inputs(
        &self,
        sid: &str,
        up: i64,
        d: opencoder_store::Delivery,
    ) -> Result<Vec<i64>, anyhow::Error> {
        self.inner.promote_inputs(sid, up, d).await
    }
    async fn promote_next_queued(&self, sid: &str) -> Result<Option<i64>, anyhow::Error> {
        self.inner.promote_next_queued(sid).await
    }
    async fn claim_next_queue(
        &self,
        sid: &str,
    ) -> Result<Option<(i64, SessionInput)>, anyhow::Error> {
        self.inner.claim_next_queue(sid).await
    }
    async fn delete_input(&self, id: i64) -> Result<(), anyhow::Error> {
        self.inner.delete_input(id).await
    }
    async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> Result<(), anyhow::Error> {
        self.inner.swap_input_order(sid, a, b).await
    }
    async fn append_events(&self, ev: &[SessionEventRecord]) -> Result<Vec<i64>, anyhow::Error> {
        self.inner.append_events(ev).await
    }
    async fn events_after(
        &self,
        sid: &str,
        s: i64,
    ) -> Result<Vec<SessionEventRecord>, anyhow::Error> {
        self.inner.events_after(sid, s).await
    }
    async fn last_event_seq(&self, sid: &str) -> Result<i64, anyhow::Error> {
        self.inner.last_event_seq(sid).await
    }
    async fn create_subagent_task(&self, r: &SubagentTaskRecord) -> Result<(), anyhow::Error> {
        self.inner.create_subagent_task(r).await
    }
    async fn complete_subagent_task(
        &self,
        id: &str,
        res: &str,
        ok: bool,
    ) -> Result<(), anyhow::Error> {
        self.inner.complete_subagent_task(id, res, ok).await
    }
    async fn list_subagent_tasks(
        &self,
        pid: &str,
    ) -> Result<Vec<SubagentTaskRecord>, anyhow::Error> {
        self.inner.list_subagent_tasks(pid).await
    }
    async fn get_subagent_task(
        &self,
        id: &str,
    ) -> Result<Option<SubagentTaskRecord>, anyhow::Error> {
        self.inner.get_subagent_task(id).await
    }
    async fn cancel_subagent_task(&self, id: &str) -> Result<(), anyhow::Error> {
        self.inner.cancel_subagent_task(id).await
    }
    async fn create_todo_workflow(
        &self,
        w: &TodoWorkflowRecord,
        items: &[TodoItemRecord],
        ev: &TodoEventRecord,
    ) -> Result<i64, anyhow::Error> {
        self.inner.create_todo_workflow(w, items, ev).await
    }
    async fn get_todo_workflow(
        &self,
        id: &str,
    ) -> Result<Option<TodoWorkflowRecord>, anyhow::Error> {
        self.inner.get_todo_workflow(id).await
    }
    async fn commit_todo_transition(
        &self,
        w: &TodoWorkflowRecord,
        items: &[TodoItemRecord],
        ev: &TodoEventRecord,
    ) -> Result<i64, anyhow::Error> {
        if ev.kind == "workflow_interrupted" && !self.fired.swap(true, Ordering::SeqCst) {
            anyhow::bail!("todo workflow generation conflict: {}", w.id);
        }
        self.inner.commit_todo_transition(w, items, ev).await
    }
    async fn todo_events_after(
        &self,
        id: &str,
        after: i64,
    ) -> Result<Vec<TodoEventRecord>, anyhow::Error> {
        self.inner.todo_events_after(id, after).await
    }
}

fn spec() -> WorkflowSpec {
    WorkflowSpec {
        schema_version: 1,
        id: "wf-retry".into(),
        name: "retry".into(),
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
            acceptance: AcceptanceSpec {
                criteria: "candidate exists".into(),
                required_tool_calls: Vec::new(),
            },
            metadata: serde_json::Value::Null,
        }],
        metadata: serde_json::Value::Null,
    }
}

#[tokio::test]
async fn interrupt_retries_past_generation_conflict() {
    let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let store: Arc<dyn Store> = Arc::new(ConflictOnceStore {
        inner: inner.clone(),
        fired: AtomicBool::new(false),
    });
    // Park the workflow non-terminally (suspended by the parent) so the
    // interrupt below targets a live, non-terminal run through the wrapper.
    let mock = Arc::new(MockChatClient::new().push_script(vec![LlmEvent::Completed {
        text: r#"{"operation":"suspend","reason":"park"}"#.into(),
        tool_calls: Vec::new(),
        usage: None,
    }]));
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime {
        store: store.clone(),
        client: mock,
        config: Config::default(),
        workdir: temp.path().to_path_buf(),
        debug_root: None,
        cancel: CancellationToken::new(),
    };
    let parked = runtime
        .run_new_with_id(spec(), "run-retry".into())
        .await
        .unwrap();
    assert_eq!(parked.status, WorkflowStatus::Suspended);

    let state = opencoder_todos::interrupt(&store, "run-retry", "retry me")
        .await
        .expect("interrupt must succeed after retrying the generation conflict");
    assert_eq!(state.status, WorkflowStatus::Suspended);
    assert_eq!(state.terminal_reason.as_deref(), Some("retry me"));

    // The failed attempt rolled back (no partial event); the retry committed
    // exactly one interrupt event.
    let events = inner.todo_events_after("run-retry", 0).await.unwrap();
    let interrupts: Vec<_> = events
        .iter()
        .filter(|event| event.kind == "workflow_interrupted")
        .collect();
    assert_eq!(interrupts.len(), 1);
    assert_eq!(interrupts[0].payload["reason"], "retry me");
}
