//! Admitter-actor store-failure path (extracted from `queue_admitter.rs`'s
//! test module to respect the file-size cap; same behavior, new home).

use std::sync::Arc;

use anyhow::Result;
use opencoder_core::Message;
use opencoder_store::{
    Delivery, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
    SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};

use super::{apply_done, spawn_admitter, submit, AdmitUiState, QUEUE_SUBMIT_FAILED_FLASH};

fn mk_input(prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: "x".into(),
        session_id: "s".into(),
        delivery: Delivery::Queue,
        prompt: prompt.into(),
        images: vec![],
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

/// Delegates everything to an inner LibsqlStore EXCEPT `admit_input`,
/// which always fails.
struct FailingAdmitStore(Arc<LibsqlStore>);

#[async_trait::async_trait]
impl Store for FailingAdmitStore {
    fn backend_name(&self) -> &'static str {
        self.0.backend_name()
    }
    async fn create_session(&self, m: &SessionMeta) -> Result<()> {
        self.0.create_session(m).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        self.0.get_session(id).await
    }
    async fn list_sessions(&self, f: &SessionFilter) -> Result<Vec<SessionListItem>> {
        self.0.list_sessions(f).await
    }
    async fn update_session(&self, id: &str, p: &SessionPatch) -> Result<()> {
        self.0.update_session(id, p).await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        self.0.delete_session(id).await
    }
    async fn clear_other_sessions(&self, k: &str) -> Result<u64> {
        self.0.clear_other_sessions(k).await
    }
    async fn append_message(&self, sid: &str, m: &Message) -> Result<i64> {
        self.0.append_message(sid, m).await
    }
    async fn append_messages(&self, sid: &str, m: &[Message]) -> Result<Vec<i64>> {
        self.0.append_messages(sid, m).await
    }
    async fn load_messages(&self, sid: &str) -> Result<Vec<Message>> {
        self.0.load_messages(sid).await
    }
    async fn last_message_seq(&self, sid: &str) -> Result<i64> {
        self.0.last_message_seq(sid).await
    }
    async fn admit_input(&self, _i: &SessionInput) -> Result<i64> {
        anyhow::bail!("admit failed")
    }
    async fn pending_inputs(&self, sid: &str, d: Delivery) -> Result<Vec<SessionInput>> {
        self.0.pending_inputs(sid, d).await
    }
    async fn promote_inputs(&self, sid: &str, up: i64, d: Delivery) -> Result<Vec<i64>> {
        self.0.promote_inputs(sid, up, d).await
    }
    async fn promote_next_queued(&self, sid: &str) -> Result<Option<i64>> {
        self.0.promote_next_queued(sid).await
    }
    async fn claim_next_queue(&self, sid: &str) -> Result<Option<(i64, SessionInput)>> {
        self.0.claim_next_queue(sid).await
    }
    async fn delete_input(&self, id: i64) -> Result<()> {
        self.0.delete_input(id).await
    }
    async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> Result<()> {
        self.0.swap_input_order(sid, a, b).await
    }
    async fn append_events(&self, ev: &[SessionEventRecord]) -> Result<Vec<i64>> {
        self.0.append_events(ev).await
    }
    async fn events_after(&self, sid: &str, s: i64) -> Result<Vec<SessionEventRecord>> {
        self.0.events_after(sid, s).await
    }
    async fn last_event_seq(&self, sid: &str) -> Result<i64> {
        self.0.last_event_seq(sid).await
    }
    async fn create_subagent_task(&self, r: &SubagentTaskRecord) -> Result<()> {
        self.0.create_subagent_task(r).await
    }
    async fn complete_subagent_task(&self, id: &str, r: &str, ok: bool) -> Result<()> {
        self.0.complete_subagent_task(id, r, ok).await
    }
    async fn list_subagent_tasks(&self, pid: &str) -> Result<Vec<SubagentTaskRecord>> {
        self.0.list_subagent_tasks(pid).await
    }
    async fn get_subagent_task(&self, id: &str) -> Result<Option<SubagentTaskRecord>> {
        self.0.get_subagent_task(id).await
    }
    async fn cancel_subagent_task(&self, id: &str) -> Result<()> {
        self.0.cancel_subagent_task(id).await
    }
}

#[tokio::test]
async fn actor_failure_path_flashes_and_removes_row() {
    let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let store: Arc<dyn Store> = Arc::new(FailingAdmitStore(inner));
    let (tx, mut done_rx) = spawn_admitter(store);
    let mut st = AdmitUiState::default();
    let mut queue_items = vec![(-100, "other".to_string())];
    let mut pending = vec![("img.png".to_string(), "alt".to_string())];
    assert!(submit(
        &tx,
        &mut st,
        &mut queue_items,
        &mut pending,
        mk_input("p"),
        "d".into()
    ));
    let done = done_rx.recv().await.unwrap();
    assert!(done.result.is_err());
    let flash = apply_done(
        &mut st,
        done,
        &mut queue_items,
        &mut vec![],
        &mut pending,
        "s",
    );
    assert_eq!(flash, Some(QUEUE_SUBMIT_FAILED_FLASH));
    assert_eq!(
        queue_items,
        vec![(-100, "other".to_string())],
        "temp row removed, others kept"
    );
    assert_eq!(pending, vec![("img.png".to_string(), "alt".to_string())]);
    assert!(st.inflight.is_empty());
}
