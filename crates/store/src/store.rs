use anyhow::Result;
use async_trait::async_trait;

use opencode_core::Message;

use crate::types::{
    ImportReport, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
    SessionMeta, SessionPatch,
};

/// Storage abstraction — the single seam that lets us swap libsql for another
/// Rust SQLite implementation later without touching upper layers.
///
/// Upper-layer code depends on `Arc<dyn Store>`; concrete impls live in
/// `libsql_store` (primary) and any future backend.
#[async_trait]
pub trait Store: Send + Sync {
    fn backend_name(&self) -> &'static str;

    async fn create_session(&self, meta: &SessionMeta) -> Result<()>;
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>>;
    async fn list_sessions(&self, filter: &SessionFilter) -> Result<Vec<SessionListItem>>;
    async fn update_session(&self, id: &str, patch: &SessionPatch) -> Result<()>;
    async fn delete_session(&self, id: &str) -> Result<()>;

    async fn append_message(&self, session_id: &str, msg: &Message) -> Result<i64>;
    async fn append_messages(&self, session_id: &str, msgs: &[Message]) -> Result<Vec<i64>>;
    async fn load_messages(&self, session_id: &str) -> Result<Vec<Message>>;
    async fn last_message_seq(&self, session_id: &str) -> Result<i64>;

    async fn admit_input(&self, input: &SessionInput) -> Result<i64>;
    async fn pending_inputs(&self, session_id: &str, delivery: crate::types::Delivery) -> Result<Vec<SessionInput>>;
    async fn promote_inputs(&self, session_id: &str, up_to_admitted_seq: i64, delivery: crate::types::Delivery) -> Result<Vec<i64>>;
    async fn promote_next_queued(&self, session_id: &str) -> Result<Option<i64>>;
    /// Atomically return the oldest pending queued input (with its prompt) and
    /// mark it promoted. Used by the runner drain at idle to consume exactly one
    /// queued follow-up per cycle.
    async fn claim_next_queue(&self, session_id: &str) -> Result<Option<SessionInput>>;

    async fn append_event(&self, event: &SessionEventRecord) -> Result<i64>;
    async fn events_after(&self, session_id: &str, after_seq: i64) -> Result<Vec<SessionEventRecord>>;

    async fn import_messages(&self, session_id: &str, msgs: &[Message]) -> Result<ImportReport> {
        let seqs = self.append_messages(session_id, msgs).await?;
        let report = ImportReport {
            sessions: if seqs.is_empty() { 0 } else { 1 },
            messages: seqs.len() as u32,
            skipped: 0,
        };
        Ok(report)
    }
}
