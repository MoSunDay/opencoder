// Method group 2; assembled before async_trait expands the complete contract.
macro_rules! store_implementation_1 {
    ($($methods:tt)*) => {
        store_implementation_2! {
            $($methods)*
    async fn create_todo_workflow(
        &self,
        workflow: &TodoWorkflowRecord,
        items: &[TodoItemRecord],
        event: &TodoEventRecord,
    ) -> Result<i64> {
        let _guard = self.db_lock.lock().await;
        todos::create(&self.conn, workflow, items, event).await
    }

    async fn get_todo_workflow(&self, id: &str) -> Result<Option<TodoWorkflowRecord>> {
        let _guard = self.db_lock.lock().await;
        todos::get(&self.conn, id).await
    }

    async fn get_todo_workflow_detail(&self, id: &str) -> Result<Option<TodoWorkflowDetail>> {
        let _guard = self.db_lock.lock().await;
        todos::get_detail(&self.conn, id).await
    }

    async fn list_todo_workflows(&self, limit: u32) -> Result<Vec<TodoWorkflowSummary>> {
        let _guard = self.db_lock.lock().await;
        todos::list(&self.conn, limit).await
    }

    async fn list_todo_items(&self, workflow_id: &str) -> Result<Vec<TodoItemRecord>> {
        let _guard = self.db_lock.lock().await;
        todos::items(&self.conn, workflow_id).await
    }
    async fn list_todo_items_page(
        &self,
        workflow_id: &str,
        after_ordinal: Option<i64>,
        limit: u32,
    ) -> Result<crate::TodoItemPage> {
        let _guard = self.db_lock.lock().await;
        todos::items_page(&self.conn, workflow_id, after_ordinal, limit).await
    }
    async fn todo_workflow_field_chunk(
        &self,
        workflow_id: &str,
        field: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<crate::PayloadChunkRecord>> {
        let _guard = self.db_lock.lock().await;
        todos::workflow_field_chunk(&self.conn, workflow_id, field, offset, max_bytes).await
    }
    async fn todo_item_field_chunk(
        &self,
        workflow_id: &str,
        todo_id: &str,
        field: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<crate::PayloadChunkRecord>> {
        let _guard = self.db_lock.lock().await;
        todos::item_field_chunk(&self.conn, workflow_id, todo_id, field, offset, max_bytes).await
    }

    async fn commit_todo_transition(
        &self,
        workflow: &TodoWorkflowRecord,
        items: &[TodoItemRecord],
        event: &TodoEventRecord,
    ) -> Result<i64> {
        let _guard = self.db_lock.lock().await;
        todos::commit(&self.conn, workflow, items, event).await
    }

    async fn todo_events_after(
        &self,
        workflow_id: &str,
        after_seq: i64,
    ) -> Result<Vec<TodoEventRecord>> {
        let _guard = self.db_lock.lock().await;
        todos::events_after(&self.conn, workflow_id, after_seq).await
    }
    async fn todo_events_page(
        &self,
        workflow_id: &str,
        after_seq: i64,
        limit: u32,
        payload_budget: usize,
    ) -> Result<TodoEventPage> {
        let _guard = self.db_lock.lock().await;
        todos::events_page(&self.conn, workflow_id, after_seq, limit, payload_budget).await
    }
    async fn todo_events_before(
        &self,
        workflow_id: &str,
        before_seq: i64,
        limit: u32,
        payload_budget: usize,
    ) -> Result<TodoEventPage> {
        let _guard = self.db_lock.lock().await;
        todos::events_before(&self.conn, workflow_id, before_seq, limit, payload_budget).await
    }
    async fn last_todo_event_seq(&self, workflow_id: &str) -> Result<i64> {
        let _guard = self.db_lock.lock().await;
        let mut rows = self
            .conn
            .query(
                "SELECT COALESCE(MAX(seq),0) FROM todo_events WHERE workflow_id=?1",
                [workflow_id],
            )
            .await?;
        Ok(rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing event watermark"))?
            .get(0)?)
    }
    async fn todo_event_payload_chunk(
        &self,
        workflow_id: &str,
        seq: i64,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<crate::PayloadChunkRecord>> {
        let _guard = self.db_lock.lock().await;
        todos::event_payload_chunk(&self.conn, workflow_id, seq, offset, max_bytes).await
    }

    async fn create_brain_capability(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
    ) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        brain::create(&self.conn, capability, eng_inputs).await
    }

    async fn update_brain_capability(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
    ) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        brain::update(&self.conn, capability, eng_inputs).await
    }

    async fn delete_brain_capability(&self, id: &str) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        brain::delete(&self.conn, id).await
    }

    async fn get_brain_capability(&self, id: &str) -> Result<Option<BrainCapabilityDetail>> {
        let _guard = self.db_lock.lock().await;
        brain::get(&self.conn, id).await
    }

    async fn list_brain_capabilities(&self) -> Result<Vec<BrainCapabilityDetail>> {
        let _guard = self.db_lock.lock().await;
        brain::list(&self.conn).await
    }

    async fn upsert_brain_vector(
        &self,
        capability_id: &str,
        dim: i64,
        model: &str,
        emb: &[u8],
        updated_at: i64,
    ) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        brain::upsert_vector(&self.conn, capability_id, dim, model, emb, updated_at).await
    }

    async fn create_brain_capability_with_vector(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
        vector: &BrainVectorWrite,
    ) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        brain::create_with_vector(&self.conn, capability, eng_inputs, vector).await
    }

    async fn update_brain_capability_with_vector(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
        vector: &BrainVectorWrite,
    ) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        brain::update_with_vector(&self.conn, capability, eng_inputs, vector).await
    }

    async fn search_brain_vectors(
        &self,
        model: &str,
        query_emb: &[u8],
        limit: u32,
    ) -> Result<Vec<BrainVectorHit>> {
        let _guard = self.db_lock.lock().await;
        brain::search(&self.conn, model, query_emb, limit).await
    }


    async fn find_user_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<crate::users::PlatformUser>> {
        let _guard = self.db_lock.lock().await;
        users::find_by_token_hash(&self.conn, token_hash).await
    }
    async fn find_user_by_name(&self, name: &str) -> Result<Option<crate::users::PlatformUser>> {
        let _guard = self.db_lock.lock().await;
        users::find_by_name(&self.conn, name).await
    }
    async fn list_users(&self) -> Result<Vec<crate::users::PlatformUser>> {
        let _guard = self.db_lock.lock().await;
        users::list(&self.conn).await
    }
    async fn create_user(
        &self,
        name: &str,
        token_hash: &str,
        role: opencoder_core::identity::Role,
        created_at: i64,
    ) -> Result<crate::users::PlatformUser> {
        let _guard = self.db_lock.lock().await;
        users::create(&self.conn, name, token_hash, role, created_at).await
    }
    async fn delete_user(&self, name: &str) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        users::delete(&self.conn, name).await
    }
    async fn delete_user_guarding_last_admin(
        &self,
        name: &str,
    ) -> Result<crate::users::GuardedDelete> {
        let _guard = self.db_lock.lock().await;
        users::delete_guarding_last_admin(&self.conn, name).await
    }
    async fn update_user_token_hash(&self, name: &str, token_hash: &str) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        users::update_token_hash(&self.conn, name, token_hash).await
    }
    async fn count_admin_users(&self) -> Result<i64> {
        let _guard = self.db_lock.lock().await;
        users::count_admins(&self.conn).await
    }
    async fn register_node(
        &self,
        name: &str,
        version: Option<&str>,
        workdir: Option<&str>,
        addr: Option<&str>,
        now_ms: i64,
    ) -> Result<NodeRecord> {
        let _guard = self.db_lock.lock().await;
        nodes::register(&self.conn, name, version, workdir, addr, now_ms).await
    }
    async fn list_nodes(&self) -> Result<Vec<NodeRecord>> {
        let _guard = self.db_lock.lock().await;
        nodes::list(&self.conn).await
    }
    async fn get_node(&self, id: &str) -> Result<Option<NodeRecord>> {
        let _guard = self.db_lock.lock().await;
        nodes::get(&self.conn, id).await
    }
    async fn delete_node(&self, id: &str) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        nodes::delete(&self.conn, id).await
    }
    async fn heartbeat_node(&self, id: &str, now_ms: i64) -> Result<Vec<String>> {
        let _guard = self.db_lock.lock().await;
        nodes::heartbeat(&self.conn, id, now_ms).await
    }
    async fn dispatch_node_task(
        &self,
        task_id: &str,
        session_id: &str,
        node_id: &str,
        title: Option<&str>,
        prompt: &str,
        agent: Option<&str>,
        model: Option<&str>,
        now_ms: i64,
    ) -> Result<NodeTaskRecord> {
        let _guard = self.db_lock.lock().await;
        node_tasks::dispatch(
            &self.conn, task_id, session_id, node_id, title, prompt, agent, model, now_ms,
        )
        .await
    }

        }
    };
}
