// Method group: conversation domain — sessions, messages, queued inputs,
// session events and subagent tasks.
//
// Each trait method is a thin adapter: clone the arguments, then run the
// domain free-function under the engine's write lease (mutations) or the
// lease-free read path (queries). Domain functions never call back into the
// trait, so the write lease cannot re-enter itself.
//
// This file is include!d by `impl_store.rs` before the assembly macro
// expands, so the imports below are module-scope for the whole impl block.
use super::RdbStore;
use crate::store::Store;
use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{Message, MessageUsage};

use crate::rdb_store::conversation as conv;
use crate::types::{
    Delivery, InputAdmission, MessageChunkPage, MessageRow, PayloadChunkRecord, SessionEventPage,
    SessionEventRecord, SessionFilter, SessionInput, SessionListItem, SessionMeta, SessionPatch,
    SubagentTaskRecord,
};

macro_rules! rdb_impl_0 {
    ($($methods:tt)*) => {
        rdb_impl_todo! {
            $($methods)*

    async fn create_session(&self, meta: &SessionMeta) -> Result<()> {
        let meta = meta.clone();
        self.db.write(move |db| conv::sessions::create(db, &meta)).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        let id = id.to_string();
        self.db.read(move |db| conv::sessions::get(db, &id)).await
    }
    async fn harness_runtime(
        &self,
        id: &str,
    ) -> Result<Option<opencoder_core::harness::HarnessRuntime>> {
        let id = id.to_string();
        self.db.read(move |db| conv::sessions::harness_runtime(db, &id)).await
    }
    async fn set_harness_runtime(
        &self,
        id: &str,
        runtime: &opencoder_core::harness::HarnessRuntime,
    ) -> Result<()> {
        let (id, runtime) = (id.to_string(), runtime.clone());
        self.db
            .write(move |db| conv::sessions::set_harness_runtime(db, &id, &runtime))
            .await
    }
    async fn list_sessions(&self, filter: &SessionFilter) -> Result<Vec<SessionListItem>> {
        let filter = filter.clone();
        self.db.read(move |db| conv::list::list(db, &filter, false)).await
    }
    async fn list_execution_sessions(
        &self,
        filter: &SessionFilter,
    ) -> Result<Vec<SessionListItem>> {
        let filter = filter.clone();
        self.db.read(move |db| conv::list::list(db, &filter, true)).await
    }
    async fn update_session(&self, id: &str, patch: &SessionPatch) -> Result<()> {
        let (id, patch) = (id.to_string(), patch.clone());
        self.db
            .write(move |db| conv::sessions::update(db, &id, &patch))
            .await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.db.write(move |db| conv::sessions::delete(db, &id)).await
    }
    async fn clear_other_sessions(&self, keep_session_id: &str) -> Result<u64> {
        let keep = keep_session_id.to_string();
        self.db.write(move |db| conv::sessions::clear_others(db, &keep)).await
    }

    async fn set_message_usage(
        &self,
        session_id: &str,
        message_id: &str,
        usage: &MessageUsage,
    ) -> Result<()> {
        let (sid, mid, usage) = (session_id.to_string(), message_id.to_string(), usage.clone());
        self.db
            .write(move |db| conv::messages::set_usage(db, &sid, &mid, &usage))
            .await
    }
    async fn append_message(&self, session_id: &str, msg: &Message) -> Result<i64> {
        let seqs = self.append_messages(session_id, std::slice::from_ref(msg)).await?;
        Ok(seqs.first().copied().unwrap_or(0))
    }
    async fn append_messages(&self, session_id: &str, msgs: &[Message]) -> Result<Vec<i64>> {
        let (sid, msgs) = (session_id.to_string(), msgs.to_vec());
        self.db
            .write(move |db| conv::messages::append_many(db, &sid, &msgs))
            .await
    }
    async fn load_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let sid = session_id.to_string();
        self.db.read(move |db| conv::messages::load(db, &sid)).await
    }
    async fn load_messages_after(&self, session_id: &str, skip_count: i64) -> Result<Vec<Message>> {
        let sid = session_id.to_string();
        self.db
            .read(move |db| conv::messages::load_after(db, &sid, skip_count))
            .await
    }
    async fn last_message_seq(&self, session_id: &str) -> Result<i64> {
        let sid = session_id.to_string();
        self.db.read(move |db| conv::messages::last_seq(db, &sid)).await
    }
    async fn load_message_rows(&self, session_id: &str) -> Result<Vec<MessageRow>> {
        let sid = session_id.to_string();
        self.db.read(move |db| conv::messages::load_rows(db, &sid)).await
    }
    async fn load_message_page(
        &self,
        session_id: &str,
        cursor: opencoder_core::fleet::MessageCursor,
        chunk_bytes: usize,
        raw_budget: usize,
    ) -> Result<MessageChunkPage> {
        let sid = session_id.to_string();
        self.db
            .read(move |db| conv::messages::load_page(db, &sid, cursor, chunk_bytes, raw_budget))
            .await
    }
    async fn load_transcript_page(
        &self,
        session_id: &str,
        cursor: opencoder_core::fleet::MessageCursor,
        chunk_bytes: usize,
        raw_budget: usize,
    ) -> Result<MessageChunkPage> {
        let sid = session_id.to_string();
        self.db
            .read(move |db| {
                conv::messages::load_transcript_page(db, &sid, cursor, chunk_bytes, raw_budget)
            })
            .await
    }

    async fn admit_input(&self, input: &SessionInput) -> Result<i64> {
        let input = input.clone();
        self.db.write(move |db| conv::inputs::admit(db, &input)).await
    }
    async fn admit_input_once(&self, input: &SessionInput) -> Result<InputAdmission> {
        let input = input.clone();
        self.db.write(move |db| conv::inputs::admit_once(db, &input)).await
    }
    async fn pending_inputs(
        &self,
        session_id: &str,
        delivery: Delivery,
    ) -> Result<Vec<SessionInput>> {
        let sid = session_id.to_string();
        self.db.read(move |db| conv::inputs::pending(db, &sid, delivery)).await
    }
    async fn promote_inputs(
        &self,
        session_id: &str,
        up_to_admitted_seq: i64,
        delivery: Delivery,
    ) -> Result<Vec<i64>> {
        let sid = session_id.to_string();
        self.db
            .write(move |db| conv::inputs::promote(db, &sid, up_to_admitted_seq, delivery))
            .await
    }
    async fn promote_next_queued(&self, session_id: &str) -> Result<Option<i64>> {
        let sid = session_id.to_string();
        self.db
            .write(move |db| conv::inputs::promote_next_queued(db, &sid))
            .await
    }
    async fn claim_next_queue(&self, session_id: &str) -> Result<Option<(i64, SessionInput)>> {
        let sid = session_id.to_string();
        self.db.write(move |db| conv::inputs::claim_next_queue(db, &sid)).await
    }
    async fn unpromote_inputs(&self, session_id: &str, seqs: &[i64]) -> Result<()> {
        let (sid, seqs) = (session_id.to_string(), seqs.to_vec());
        self.db.write(move |db| conv::inputs::unpromote(db, &sid, &seqs)).await
    }
    async fn mark_inputs_recorded(&self, session_id: &str, seqs: &[i64]) -> Result<()> {
        let (sid, seqs) = (session_id.to_string(), seqs.to_vec());
        self.db.write(move |db| conv::inputs::mark_recorded(db, &sid, &seqs)).await
    }
    async fn recover_orphan_inputs(&self, session_id: &str) -> Result<u64> {
        let sid = session_id.to_string();
        self.db.write(move |db| conv::inputs::recover_orphans(db, &sid)).await
    }
    async fn delete_input(&self, input_id: i64) -> Result<()> {
        self.db.write(move |db| conv::inputs::delete_input(db, input_id)).await
    }
    async fn swap_input_order(&self, session_id: &str, seq_a: i64, seq_b: i64) -> Result<()> {
        let sid = session_id.to_string();
        self.db
            .write(move |db| conv::inputs::swap_input_order(db, &sid, seq_a, seq_b))
            .await
    }

    async fn append_events(&self, events: &[SessionEventRecord]) -> Result<Vec<i64>> {
        let events = events.to_vec();
        self.db.write(move |db| conv::events::append_many(db, &events)).await
    }
    async fn events_after(
        &self,
        session_id: &str,
        after_seq: i64,
    ) -> Result<Vec<SessionEventRecord>> {
        let sid = session_id.to_string();
        self.db.read(move |db| conv::events::after(db, &sid, after_seq)).await
    }
    async fn events_page(
        &self,
        session_id: &str,
        after_seq: i64,
        limit: u32,
        payload_budget: usize,
    ) -> Result<SessionEventPage> {
        let sid = session_id.to_string();
        self.db
            .read(move |db| conv::events::page(db, &sid, after_seq, limit, payload_budget))
            .await
    }
    async fn event_payload_chunk(
        &self,
        session_id: &str,
        seq: i64,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<PayloadChunkRecord>> {
        let sid = session_id.to_string();
        self.db
            .read(move |db| conv::events::payload_chunk(db, &sid, seq, offset, max_bytes))
            .await
    }
    async fn last_event_seq(&self, session_id: &str) -> Result<i64> {
        let sid = session_id.to_string();
        self.db.read(move |db| conv::events::last_seq(db, &sid)).await
    }

    async fn create_subagent_task(&self, record: &SubagentTaskRecord) -> Result<()> {
        let record = record.clone();
        self.db.write(move |db| conv::subagent_tasks::create(db, &record)).await
    }
    async fn complete_subagent_task(&self, task_id: &str, result: &str, ok: bool) -> Result<()> {
        let (tid, result) = (task_id.to_string(), result.to_string());
        self.db
            .write(move |db| conv::subagent_tasks::complete(db, &tid, &result, ok))
            .await
    }
    async fn list_subagent_tasks(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<SubagentTaskRecord>> {
        let pid = parent_session_id.to_string();
        self.db.read(move |db| conv::subagent_tasks::list(db, &pid)).await
    }
    async fn get_subagent_task(&self, task_id: &str) -> Result<Option<SubagentTaskRecord>> {
        let tid = task_id.to_string();
        self.db.read(move |db| conv::subagent_tasks::get(db, &tid)).await
    }
    async fn cancel_subagent_task(&self, task_id: &str) -> Result<()> {
        let tid = task_id.to_string();
        self.db.write(move |db| conv::subagent_tasks::cancel(db, &tid)).await
    }
        }
    };
}
