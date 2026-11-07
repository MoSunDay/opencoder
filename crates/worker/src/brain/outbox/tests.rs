use super::*;
use crate::DirectoryLayout;
use opencoder_core::Message;
use opencoder_store::{
    Delivery, RdbStore, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
    SessionMeta, SessionPatch, SubagentTaskRecord,
};
use std::sync::atomic::{AtomicBool, Ordering};

/// Store decorator whose ACK write can be failed on demand, so the reader
/// bridge's write ordering stays testable without a second database engine.
struct FlakyAckStore {
    inner: Arc<RdbStore>,
    fail: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl Store for FlakyAckStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create_session(&self, meta: &SessionMeta) -> anyhow::Result<()> {
        self.inner.create_session(meta).await
    }
    async fn get_session(&self, id: &str) -> anyhow::Result<Option<SessionMeta>> {
        self.inner.get_session(id).await
    }
    async fn list_sessions(&self, filter: &SessionFilter) -> anyhow::Result<Vec<SessionListItem>> {
        self.inner.list_sessions(filter).await
    }
    async fn update_session(&self, id: &str, patch: &SessionPatch) -> anyhow::Result<()> {
        self.inner.update_session(id, patch).await
    }
    async fn delete_session(&self, id: &str) -> anyhow::Result<()> {
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, keep: &str) -> anyhow::Result<u64> {
        self.inner.clear_other_sessions(keep).await
    }
    async fn append_message(&self, session_id: &str, msg: &Message) -> anyhow::Result<i64> {
        self.inner.append_message(session_id, msg).await
    }
    async fn append_messages(
        &self,
        session_id: &str,
        msgs: &[Message],
    ) -> anyhow::Result<Vec<i64>> {
        self.inner.append_messages(session_id, msgs).await
    }
    async fn load_messages(&self, session_id: &str) -> anyhow::Result<Vec<Message>> {
        self.inner.load_messages(session_id).await
    }
    async fn last_message_seq(&self, session_id: &str) -> anyhow::Result<i64> {
        self.inner.last_message_seq(session_id).await
    }
    async fn admit_input(&self, input: &SessionInput) -> anyhow::Result<i64> {
        self.inner.admit_input(input).await
    }
    async fn pending_inputs(
        &self,
        session_id: &str,
        delivery: Delivery,
    ) -> anyhow::Result<Vec<SessionInput>> {
        self.inner.pending_inputs(session_id, delivery).await
    }
    async fn promote_inputs(
        &self,
        session_id: &str,
        up_to: i64,
        delivery: Delivery,
    ) -> anyhow::Result<Vec<i64>> {
        self.inner.promote_inputs(session_id, up_to, delivery).await
    }
    async fn promote_next_queued(&self, session_id: &str) -> anyhow::Result<Option<i64>> {
        self.inner.promote_next_queued(session_id).await
    }
    async fn claim_next_queue(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Option<(i64, SessionInput)>> {
        self.inner.claim_next_queue(session_id).await
    }
    async fn delete_input(&self, input_id: i64) -> anyhow::Result<()> {
        self.inner.delete_input(input_id).await
    }
    async fn swap_input_order(
        &self,
        session_id: &str,
        seq_a: i64,
        seq_b: i64,
    ) -> anyhow::Result<()> {
        self.inner.swap_input_order(session_id, seq_a, seq_b).await
    }
    async fn append_events(&self, events: &[SessionEventRecord]) -> anyhow::Result<Vec<i64>> {
        self.inner.append_events(events).await
    }
    async fn events_after(
        &self,
        session_id: &str,
        after_seq: i64,
    ) -> anyhow::Result<Vec<SessionEventRecord>> {
        self.inner.events_after(session_id, after_seq).await
    }
    async fn last_event_seq(&self, session_id: &str) -> anyhow::Result<i64> {
        self.inner.last_event_seq(session_id).await
    }
    async fn create_subagent_task(&self, record: &SubagentTaskRecord) -> anyhow::Result<()> {
        self.inner.create_subagent_task(record).await
    }
    async fn complete_subagent_task(
        &self,
        task_id: &str,
        result: &str,
        ok: bool,
    ) -> anyhow::Result<()> {
        self.inner.complete_subagent_task(task_id, result, ok).await
    }
    async fn list_subagent_tasks(&self, parent: &str) -> anyhow::Result<Vec<SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(parent).await
    }
    async fn get_subagent_task(&self, task_id: &str) -> anyhow::Result<Option<SubagentTaskRecord>> {
        self.inner.get_subagent_task(task_id).await
    }
    async fn cancel_subagent_task(&self, task_id: &str) -> anyhow::Result<()> {
        self.inner.cancel_subagent_task(task_id).await
    }

    async fn report_ack(&self, ack: &ReportAck) -> anyhow::Result<()> {
        if self.fail.load(Ordering::SeqCst) {
            anyhow::bail!("injected ACK failure");
        }
        self.inner.report_ack(ack).await
    }
    async fn report_acks(&self) -> anyhow::Result<Vec<ReportAck>> {
        self.inner.report_acks().await
    }
    async fn reporting_recover(&self) -> anyhow::Result<()> {
        self.inner.reporting_recover().await
    }
    async fn reporting_format(&self, reader: u32, writer: u32) -> anyhow::Result<()> {
        self.inner.reporting_format(reader, writer).await
    }
}

#[tokio::test]
async fn reader_bridge_persists_legacy_ack_before_database_and_repairs_interrupted_write() {
    for legacy in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let layout = DirectoryLayout::new(dir.path().join("journal"), None).unwrap();
        let mut journal = Journal::load(layout.clone()).unwrap();
        let record = serde_json::from_value(json!({"assignment": {
            "index":{"id":"agent-child","kind":"agent","node_id":"node","created_at":1,"status":"done"},
            "request":{"id":"agent-child","kind":"agent","input":{"schema_version":7,
                "payload":"unchanged", "brain_layered":{"parent":{"run_id":"brain-parent","operation_id":"op","node_id":"step","layer":1}}}}},
            "result":null,"error":null,"events":[{"kind":"done","data":{},"ts":1,"seq":1}]})).unwrap();
        journal.save(record).unwrap();
        let path = layout
            .record_path(ExecutionKind::Agent, "agent-child")
            .unwrap();
        let original = std::fs::read(&path).unwrap();
        let fail = Arc::new(AtomicBool::new(true));
        let store: Arc<dyn Store> = Arc::new(FlakyAckStore {
            inner: Arc::new(RdbStore::open_memory().await.unwrap()),
            fail: fail.clone(),
        });
        let ack = ReportAck {
            execution_id: "agent-child".into(),
            action: "layered_terminal".into(),
            item: String::new(),
            cursor: 1,
        };
        assert!(save_ack_with_format(&store, &mut journal, &ack, legacy)
            .await
            .is_err());
        let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved["assignment"]["request"]["input"]["payload"],
            "unchanged"
        );
        if legacy {
            assert_eq!(
                saved["annotations"]["brain_layered_ack"], 1,
                "a crash cannot leave only the new ACK representation durable during the bridge"
            );
        } else {
            assert_eq!(std::fs::read(&path).unwrap(), original);
            assert!(
                journal.has_terminal_reports(),
                "a failed durable ACK must remain pending"
            );
        }
        fail.store(false, Ordering::SeqCst);
        let mut recovered = Journal::load(layout).unwrap();
        initialize(&store, &mut recovered).await.unwrap();
        if legacy {
            assert_eq!(store.report_acks().await.unwrap()[0].cursor, 1);
            assert!(!recovered.has_terminal_reports());
        } else {
            assert!(store.report_acks().await.unwrap().is_empty());
            assert!(recovered.has_terminal_reports());
            save_ack_with_format(&store, &mut recovered, &ack, false)
                .await
                .unwrap();
            assert!(!recovered.has_terminal_reports());
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
    }
}
