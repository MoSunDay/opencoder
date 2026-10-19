macro_rules! store_implementation_0 {
    ($($methods:tt)*) => {
        store_implementation_1! {
            $($methods)*
    async fn brain_layered(
        &self,
        id: &str,
    ) -> Result<Option<opencoder_core::brain::layered::LayeredSnapshot>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        super::brain_layered::load(&conn, id).await

        }).await
    }
    async fn commit_brain_layered(
        &self,
        change: &opencoder_core::brain::layered::LayeredChange,
    ) -> Result<opencoder_core::brain::layered::LayeredSnapshot> {
        let change = change.to_owned();
        self.db.write(move |conn| async move {
            let change = &change;

        super::brain_layered::commit(&conn, change).await

        }).await
    }
    async fn brain_layered_events(
        &self,
        id: &str,
        after: u64,
        limit: u32,
    ) -> Result<Vec<opencoder_core::brain::layered::LayeredEvent>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        super::brain_layered::page(&conn, id, after, limit).await

        }).await
    }
    async fn set_message_usage(
        &self,
        session_id: &str,
        message_id: &str,
        usage: &opencoder_core::MessageUsage,
    ) -> Result<()> {
        let session_id = session_id.to_owned();
        let message_id = message_id.to_owned();
        let usage = usage.to_owned();
        self.db.write(move |conn| async move {
            let session_id = session_id.as_str();
            let message_id = message_id.as_str();
            let usage = &usage;

        messages::set_usage(&conn, session_id, message_id, usage).await

        }).await
    }
    async fn harness_runtime(
        &self,
        id: &str,
    ) -> Result<Option<opencoder_core::harness::HarnessRuntime>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        sessions::harness_runtime(&conn, id).await

        }).await
    }
    async fn set_harness_runtime(
        &self,
        id: &str,
        runtime: &opencoder_core::harness::HarnessRuntime,
    ) -> Result<()> {
        let id = id.to_owned();
        let runtime = runtime.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();
            let runtime = &runtime;

        sessions::set_harness_runtime(&conn, id, runtime).await

        }).await
    }
    async fn create_session(&self, meta: &SessionMeta) -> Result<()> {
        let meta = meta.to_owned();
        self.db.write(move |conn| async move {
            let meta = &meta;

        sessions::create(&conn, meta).await

        }).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        sessions::get(&conn, id).await

        }).await
    }
    async fn list_sessions(&self, filter: &SessionFilter) -> Result<Vec<SessionListItem>> {
        let filter = filter.to_owned();
        self.db.read(move |conn| async move {
            let filter = &filter;

        sessions::list(&conn, filter).await

        }).await
    }
    async fn list_execution_sessions(
        &self,
        filter: &SessionFilter,
    ) -> Result<Vec<SessionListItem>> {
        let filter = filter.to_owned();
        self.db.read(move |conn| async move {
            let filter = &filter;

        sessions::list_execution_sessions(&conn, filter).await

        }).await
    }
    async fn update_session(&self, id: &str, patch: &SessionPatch) -> Result<()> {
        let id = id.to_owned();
        let patch = patch.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();
            let patch = &patch;

        sessions::update(&conn, id, patch).await

        }).await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();

        sessions::delete(&conn, id).await

        }).await
    }
    async fn clear_other_sessions(&self, keep_session_id: &str) -> Result<u64> {
        let keep_session_id = keep_session_id.to_owned();
        self.db.write(move |conn| async move {
            let keep_session_id = keep_session_id.as_str();

        sessions::clear_others(&conn, keep_session_id).await

        }).await
    }
    async fn append_message(&self, session_id: &str, msg: &opencoder_core::Message) -> Result<i64> {
        let session_id = session_id.to_owned();
        let msg = msg.to_owned();
        self.db.write(move |conn| async move {
            let session_id = session_id.as_str();
            let msg = &msg;

        messages::append(&conn, session_id, msg).await

        }).await
    }
    async fn append_messages(
        &self,
        session_id: &str,
        msgs: &[opencoder_core::Message],
    ) -> Result<Vec<i64>> {
        let session_id = session_id.to_owned();
        let msgs = msgs.to_owned();
        self.db.write(move |conn| async move {
            let session_id = session_id.as_str();
            let msgs = &msgs;

        messages::append_many(&conn, session_id, msgs).await

        }).await
    }
    async fn load_messages(&self, session_id: &str) -> Result<Vec<opencoder_core::Message>> {
        let session_id = session_id.to_owned();
        self.db.read(move |conn| async move {
            let session_id = session_id.as_str();

        messages::load(&conn, session_id).await

        }).await
    }
    async fn load_messages_after(
        &self,
        session_id: &str,
        skip_count: i64,
    ) -> Result<Vec<opencoder_core::Message>> {
        let session_id = session_id.to_owned();
        self.db.read(move |conn| async move {
            let session_id = session_id.as_str();

        messages::load_after(&conn, session_id, skip_count).await

        }).await
    }
    async fn last_message_seq(&self, session_id: &str) -> Result<i64> {
        let session_id = session_id.to_owned();
        self.db.read(move |conn| async move {
            let session_id = session_id.as_str();

        messages::last_seq(&conn, session_id).await

        }).await
    }
    async fn load_message_rows(&self, session_id: &str) -> Result<Vec<MessageRow>> {
        let session_id = session_id.to_owned();
        self.db.read(move |conn| async move {
            let session_id = session_id.as_str();

        messages::load_rows(&conn, session_id).await

        }).await
    }
    async fn load_message_page(
        &self,
        session_id: &str,
        cursor: opencoder_core::fleet::MessageCursor,
        chunk_bytes: usize,
        raw_budget: usize,
    ) -> Result<MessageChunkPage> {
        let session_id = session_id.to_owned();
        self.db.read(move |conn| async move {
            let session_id = session_id.as_str();

        messages::load_page(&conn, session_id, cursor, chunk_bytes, raw_budget).await

        }).await
    }
    async fn load_transcript_page(
        &self, session_id: &str, cursor: opencoder_core::fleet::MessageCursor,
        chunk_bytes: usize, raw_budget: usize,
    ) -> Result<MessageChunkPage> {
        let session_id = session_id.to_owned();
        self.db.read(move |conn| async move {
            let session_id = session_id.as_str();

        messages::load_transcript_page(&conn, session_id, cursor, chunk_bytes, raw_budget).await

        }).await
    }
    async fn admit_input(&self, input: &SessionInput) -> Result<i64> {
        let input = input.to_owned();
        self.db.write(move |conn| async move {
            let input = &input;

        inputs::admit(&conn, input).await

        }).await
    }
    async fn admit_input_once(&self, input: &SessionInput) -> Result<InputAdmission> {
        let input = input.to_owned();
        self.db.write(move |conn| async move {
            let input = &input;

        inputs::admit_once(&conn, input).await

        }).await
    }
    async fn pending_inputs(
        &self,
        session_id: &str,
        delivery: Delivery,
    ) -> Result<Vec<SessionInput>> {
        let session_id = session_id.to_owned();
        self.db.read(move |conn| async move {
            let session_id = session_id.as_str();

        inputs::pending(&conn, session_id, delivery).await

        }).await
    }
    async fn promote_inputs(
        &self,
        session_id: &str,
        up_to_admitted_seq: i64,
        delivery: Delivery,
    ) -> Result<Vec<i64>> {
        let session_id = session_id.to_owned();
        self.db.write(move |conn| async move {
            let session_id = session_id.as_str();

        inputs::promote(&conn, session_id, up_to_admitted_seq, delivery).await

        }).await
    }
    async fn promote_next_queued(&self, session_id: &str) -> Result<Option<i64>> {
        let session_id = session_id.to_owned();
        self.db.write(move |conn| async move {
            let session_id = session_id.as_str();

        inputs::promote_next_queued(&conn, session_id).await

        }).await
    }
    async fn claim_next_queue(&self, session_id: &str) -> Result<Option<(i64, SessionInput)>> {
        let session_id = session_id.to_owned();
        self.db.write(move |conn| async move {
            let session_id = session_id.as_str();

        inputs::claim_next_queue(&conn, session_id).await

        }).await
    }
    async fn unpromote_inputs(&self, session_id: &str, seqs: &[i64]) -> Result<()> {
        let session_id = session_id.to_owned();
        let seqs = seqs.to_owned();
        self.db.write(move |conn| async move {
            let session_id = session_id.as_str();
            let seqs = &seqs;

        inputs::unpromote(&conn, session_id, seqs).await

        }).await
    }
        }
    };
}
