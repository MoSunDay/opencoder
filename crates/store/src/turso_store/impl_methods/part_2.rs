macro_rules! store_implementation_1 {
    ($($methods:tt)*) => {
        store_implementation_2! {
            $($methods)*
    async fn mark_inputs_recorded(&self, session_id: &str, seqs: &[i64]) -> Result<()> {
        let session_id = session_id.to_owned();
        let seqs = seqs.to_owned();
        self.db.write(move |conn| async move {
            let session_id = session_id.as_str();
            let seqs = &seqs;

        inputs::mark_recorded(&conn, session_id, seqs).await

        }).await
    }
    async fn recover_orphan_inputs(&self, session_id: &str) -> Result<u64> {
        let session_id = session_id.to_owned();
        self.db.write(move |conn| async move {
            let session_id = session_id.as_str();

        inputs::recover_orphans(&conn, session_id).await

        }).await
    }
    async fn delete_input(&self, input_id: i64) -> Result<()> {
        self.db.write(move |conn| async move {

        inputs::delete_input(&conn, input_id).await

        }).await
    }
    async fn swap_input_order(&self, session_id: &str, seq_a: i64, seq_b: i64) -> Result<()> {
        let session_id = session_id.to_owned();
        self.db.write(move |conn| async move {
            let session_id = session_id.as_str();

        inputs::swap_input_order(&conn, session_id, seq_a, seq_b).await

        }).await
    }
    async fn append_events(&self, events: &[SessionEventRecord]) -> Result<Vec<i64>> {
        let events = events.to_owned();
        self.db.write(move |conn| async move {
            let events = &events;

        events::append_many(&conn, events).await

        }).await
    }
    async fn events_after(
        &self,
        session_id: &str,
        after_seq: i64,
    ) -> Result<Vec<SessionEventRecord>> {
        let session_id = session_id.to_owned();
        self.db.read(move |conn| async move {
            let session_id = session_id.as_str();

        events::after(&conn, session_id, after_seq).await

        }).await
    }
    async fn events_page(
        &self,
        session_id: &str,
        after_seq: i64,
        limit: u32,
        payload_budget: usize,
    ) -> Result<SessionEventPage> {
        let session_id = session_id.to_owned();
        self.db.read(move |conn| async move {
            let session_id = session_id.as_str();

        events::page(&conn, session_id, after_seq, limit, payload_budget).await

        }).await
    }
    async fn event_payload_chunk(
        &self,
        session_id: &str,
        seq: i64,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<crate::PayloadChunkRecord>> {
        let session_id = session_id.to_owned();
        self.db.read(move |conn| async move {
            let session_id = session_id.as_str();

        events::payload_chunk(&conn, session_id, seq, offset, max_bytes).await

        }).await
    }
    async fn last_event_seq(&self, session_id: &str) -> Result<i64> {
        let session_id = session_id.to_owned();
        self.db.read(move |conn| async move {
            let session_id = session_id.as_str();

        events::last_seq(&conn, session_id).await

        }).await
    }
    async fn dag_step_snapshot(
        &self,
        id: &str,
    ) -> Result<crate::store::dag_snapshot::DagStepSnapshot> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        events::dag_snapshot(&conn, id).await

        }).await
    }
    async fn create_subagent_task(&self, record: &SubagentTaskRecord) -> Result<()> {
        let record = record.to_owned();
        self.db.write(move |conn| async move {
            let record = &record;

        subagent_tasks::create(&conn, record).await

        }).await
    }
    async fn complete_subagent_task(&self, task_id: &str, result: &str, ok: bool) -> Result<()> {
        let task_id = task_id.to_owned();
        let result = result.to_owned();
        self.db.write(move |conn| async move {
            let task_id = task_id.as_str();
            let result = result.as_str();

        subagent_tasks::complete(&conn, task_id, result, ok).await

        }).await
    }
    async fn list_subagent_tasks(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<SubagentTaskRecord>> {
        let parent_session_id = parent_session_id.to_owned();
        self.db.read(move |conn| async move {
            let parent_session_id = parent_session_id.as_str();

        subagent_tasks::list(&conn, parent_session_id).await

        }).await
    }
    async fn get_subagent_task(&self, task_id: &str) -> Result<Option<SubagentTaskRecord>> {
        let task_id = task_id.to_owned();
        self.db.read(move |conn| async move {
            let task_id = task_id.as_str();

        subagent_tasks::get_by_task_id(&conn, task_id).await

        }).await
    }
    async fn cancel_subagent_task(&self, task_id: &str) -> Result<()> {
        let task_id = task_id.to_owned();
        self.db.write(move |conn| async move {
            let task_id = task_id.as_str();

        subagent_tasks::cancel(&conn, task_id).await

        }).await
    }
    async fn create_todo_workflow(
        &self,
        workflow: &TodoWorkflowRecord,
        items: &[TodoItemRecord],
        event: &TodoEventRecord,
    ) -> Result<i64> {
        let workflow = workflow.to_owned();
        let items = items.to_owned();
        let event = event.to_owned();
        self.db.write(move |conn| async move {
            let workflow = &workflow;
            let items = &items;
            let event = &event;

        todos::create(&conn, workflow, items, event).await

        }).await
    }
    async fn get_todo_workflow(&self, id: &str) -> Result<Option<TodoWorkflowRecord>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        todos::get(&conn, id).await

        }).await
    }
    async fn get_todo_workflow_detail(&self, id: &str) -> Result<Option<TodoWorkflowDetail>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        todos::get_detail(&conn, id).await

        }).await
    }
    async fn list_todo_workflows(&self, limit: u32) -> Result<Vec<TodoWorkflowSummary>> {
        self.db.read(move |conn| async move {

        todos::list(&conn, limit).await

        }).await
    }
    async fn list_todo_items(&self, workflow_id: &str) -> Result<Vec<TodoItemRecord>> {
        let workflow_id = workflow_id.to_owned();
        self.db.read(move |conn| async move {
            let workflow_id = workflow_id.as_str();

        todos::items(&conn, workflow_id).await

        }).await
    }
    async fn list_todo_items_page(
        &self,
        workflow_id: &str,
        after_ordinal: Option<i64>,
        limit: u32,
    ) -> Result<crate::TodoItemPage> {
        let workflow_id = workflow_id.to_owned();
        self.db.read(move |conn| async move {
            let workflow_id = workflow_id.as_str();

        todos::items_page(&conn, workflow_id, after_ordinal, limit).await

        }).await
    }
    async fn todo_workflow_field_chunk(
        &self,
        workflow_id: &str,
        field: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<crate::PayloadChunkRecord>> {
        let workflow_id = workflow_id.to_owned();
        let field = field.to_owned();
        self.db.read(move |conn| async move {
            let workflow_id = workflow_id.as_str();
            let field = field.as_str();

        todos::workflow_field_chunk(&conn, workflow_id, field, offset, max_bytes).await

        }).await
    }
    async fn todo_item_field_chunk(
        &self,
        workflow_id: &str,
        todo_id: &str,
        field: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<crate::PayloadChunkRecord>> {
        let workflow_id = workflow_id.to_owned();
        let todo_id = todo_id.to_owned();
        let field = field.to_owned();
        self.db.read(move |conn| async move {
            let workflow_id = workflow_id.as_str();
            let todo_id = todo_id.as_str();
            let field = field.as_str();

        todos::item_field_chunk(&conn, workflow_id, todo_id, field, offset, max_bytes).await

        }).await
    }
    async fn commit_todo_transition(
        &self,
        workflow: &TodoWorkflowRecord,
        items: &[TodoItemRecord],
        event: &TodoEventRecord,
    ) -> Result<i64> {
        let workflow = workflow.to_owned();
        let items = items.to_owned();
        let event = event.to_owned();
        self.db.write(move |conn| async move {
            let workflow = &workflow;
            let items = &items;
            let event = &event;

        todos::commit(&conn, workflow, items, event).await

        }).await
    }
    async fn todo_events_after(
        &self,
        workflow_id: &str,
        after_seq: i64,
    ) -> Result<Vec<TodoEventRecord>> {
        let workflow_id = workflow_id.to_owned();
        self.db.read(move |conn| async move {
            let workflow_id = workflow_id.as_str();

        todos::events_after(&conn, workflow_id, after_seq).await

        }).await
    }
    async fn todo_events_page(
        &self,
        workflow_id: &str,
        after_seq: i64,
        limit: u32,
        payload_budget: usize,
    ) -> Result<TodoEventPage> {
        let workflow_id = workflow_id.to_owned();
        self.db.read(move |conn| async move {
            let workflow_id = workflow_id.as_str();

        todos::events_page(&conn, workflow_id, after_seq, limit, payload_budget).await

        }).await
    }
        }
    };
}
