// Method group 1; assembled before async_trait expands the complete contract.
macro_rules! store_implementation_0 {
    ($($methods:tt)*) => {
        store_implementation_1! {
            $($methods)*

    async fn brain_layered(
        &self,
        id: &str,
    ) -> Result<Option<opencoder_core::brain::layered::LayeredSnapshot>> {
        let _guard = self.db_lock.lock().await;
        super::brain_layered::load(&self.conn, id).await
    }
    async fn commit_brain_layered(
        &self,
        change: &opencoder_core::brain::layered::LayeredChange,
    ) -> Result<opencoder_core::brain::layered::LayeredSnapshot> {
        let _guard = self.db_lock.lock().await;
        super::brain_layered::commit(&self.conn, change).await
    }
    async fn brain_layered_events(
        &self,
        id: &str,
        after: u64,
        limit: u32,
    ) -> Result<Vec<opencoder_core::brain::layered::LayeredEvent>> {
        let _guard = self.db_lock.lock().await;
        super::brain_layered::page(&self.conn, id, after, limit).await
    }

    async fn set_message_usage(
        &self,
        session_id: &str,
        message_id: &str,
        usage: &opencoder_core::MessageUsage,
    ) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        messages::set_usage(&self.conn, session_id, message_id, usage).await
    }

    async fn harness_runtime(
        &self,
        id: &str,
    ) -> Result<Option<opencoder_core::harness::HarnessRuntime>> {
        let _guard = self.db_lock.lock().await;
        sessions::harness_runtime(&self.conn, id).await
    }
    async fn set_harness_runtime(
        &self,
        id: &str,
        runtime: &opencoder_core::harness::HarnessRuntime,
    ) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        sessions::set_harness_runtime(&self.conn, id, runtime).await
    }
    fn backend_name(&self) -> &'static str {
        "libsql"
    }

    async fn create_session(&self, meta: &SessionMeta) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        sessions::create(&conn, meta).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        sessions::get(&conn, id).await
    }
    async fn list_sessions(&self, filter: &SessionFilter) -> Result<Vec<SessionListItem>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        sessions::list(&conn, filter).await
    }
    async fn list_execution_sessions(
        &self,
        filter: &SessionFilter,
    ) -> Result<Vec<SessionListItem>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        sessions::list_execution_sessions(&conn, filter).await
    }
    async fn update_session(&self, id: &str, patch: &SessionPatch) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        sessions::update(&conn, id, patch).await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        sessions::delete(&conn, id).await
    }
    async fn clear_other_sessions(&self, keep_session_id: &str) -> Result<u64> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        sessions::clear_others(&conn, keep_session_id).await
    }

    async fn append_message(&self, session_id: &str, msg: &opencoder_core::Message) -> Result<i64> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        messages::append(&conn, session_id, msg).await
    }
    async fn append_messages(
        &self,
        session_id: &str,
        msgs: &[opencoder_core::Message],
    ) -> Result<Vec<i64>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        messages::append_many(&conn, session_id, msgs).await
    }
    async fn load_messages(&self, session_id: &str) -> Result<Vec<opencoder_core::Message>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        messages::load(&conn, session_id).await
    }
    async fn load_messages_after(
        &self,
        session_id: &str,
        skip_count: i64,
    ) -> Result<Vec<opencoder_core::Message>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        messages::load_after(&conn, session_id, skip_count).await
    }
    async fn last_message_seq(&self, session_id: &str) -> Result<i64> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        messages::last_seq(&conn, session_id).await
    }
    async fn load_message_rows(&self, session_id: &str) -> Result<Vec<MessageRow>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        messages::load_rows(&conn, session_id).await
    }
    async fn load_message_page(
        &self,
        session_id: &str,
        cursor: opencoder_core::fleet::MessageCursor,
        chunk_bytes: usize,
        raw_budget: usize,
    ) -> Result<MessageChunkPage> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        messages::load_page(&conn, session_id, cursor, chunk_bytes, raw_budget).await
    }

    async fn admit_input(&self, input: &SessionInput) -> Result<i64> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        inputs::admit(&conn, input).await
    }
    async fn admit_input_once(&self, input: &SessionInput) -> Result<InputAdmission> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        inputs::admit_once(&conn, input).await
    }
    async fn pending_inputs(
        &self,
        session_id: &str,
        delivery: Delivery,
    ) -> Result<Vec<SessionInput>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        inputs::pending(&conn, session_id, delivery).await
    }
    async fn promote_inputs(
        &self,
        session_id: &str,
        up_to_admitted_seq: i64,
        delivery: Delivery,
    ) -> Result<Vec<i64>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        inputs::promote(&conn, session_id, up_to_admitted_seq, delivery).await
    }
    async fn promote_next_queued(&self, session_id: &str) -> Result<Option<i64>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        inputs::promote_next_queued(&conn, session_id).await
    }
    async fn claim_next_queue(&self, session_id: &str) -> Result<Option<(i64, SessionInput)>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        inputs::claim_next_queue(&conn, session_id).await
    }
    async fn unpromote_inputs(&self, session_id: &str, seqs: &[i64]) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        inputs::unpromote(&conn, session_id, seqs).await
    }
    async fn mark_inputs_recorded(&self, session_id: &str, seqs: &[i64]) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        inputs::mark_recorded(&conn, session_id, seqs).await
    }
    async fn recover_orphan_inputs(&self, session_id: &str) -> Result<u64> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        inputs::recover_orphans(&conn, session_id).await
    }
    async fn delete_input(&self, input_id: i64) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        inputs::delete_input(&conn, input_id).await
    }
    async fn swap_input_order(&self, session_id: &str, seq_a: i64, seq_b: i64) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        inputs::swap_input_order(&conn, session_id, seq_a, seq_b).await
    }

    async fn append_events(&self, events: &[SessionEventRecord]) -> Result<Vec<i64>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        events::append_many(&conn, events).await
    }
    async fn events_after(
        &self,
        session_id: &str,
        after_seq: i64,
    ) -> Result<Vec<SessionEventRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        events::after(&conn, session_id, after_seq).await
    }
    async fn events_page(
        &self,
        session_id: &str,
        after_seq: i64,
        limit: u32,
        payload_budget: usize,
    ) -> Result<SessionEventPage> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        events::page(&conn, session_id, after_seq, limit, payload_budget).await
    }
    async fn event_payload_chunk(
        &self,
        session_id: &str,
        seq: i64,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<crate::PayloadChunkRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        events::payload_chunk(&conn, session_id, seq, offset, max_bytes).await
    }
    async fn last_event_seq(&self, session_id: &str) -> Result<i64> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        events::last_seq(&conn, session_id).await
    }

    async fn dag_step_snapshot(
        &self,
        id: &str,
    ) -> Result<crate::store::dag_snapshot::DagStepSnapshot> {
        let _guard = self.db_lock.lock().await;
        events::dag_snapshot(&self.conn, id).await
    }

    async fn create_subagent_task(&self, record: &SubagentTaskRecord) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        subagent_tasks::create(&conn, record).await
    }
    async fn complete_subagent_task(&self, task_id: &str, result: &str, ok: bool) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        subagent_tasks::complete(&conn, task_id, result, ok).await
    }
    async fn list_subagent_tasks(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<SubagentTaskRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        subagent_tasks::list(&conn, parent_session_id).await
    }
    async fn get_subagent_task(&self, task_id: &str) -> Result<Option<SubagentTaskRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        subagent_tasks::get_by_task_id(&conn, task_id).await
    }
    async fn cancel_subagent_task(&self, task_id: &str) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        subagent_tasks::cancel(&conn, task_id).await
    }


        }
    };
}
