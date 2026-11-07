// Method group: todo domain — workflows, items, transitions and the
// workflow event log. Adapters follow part_1_conversation: clone args,
// run the domain free-function under the write lease (mutations) or the
// lease-free read path (queries).
use crate::rdb_store::todo;
use crate::{
    TodoEventPage, TodoEventRecord, TodoItemRecord, TodoWorkflowDetail, TodoWorkflowRecord,
    TodoWorkflowSummary,
};

macro_rules! rdb_impl_todo {
    ($($methods:tt)*) => {
        rdb_impl_execution! {
            $($methods)*

    async fn create_todo_workflow(
        &self,
        workflow: &TodoWorkflowRecord,
        items: &[TodoItemRecord],
        event: &TodoEventRecord,
    ) -> Result<i64> {
        let (workflow, items, event) = (workflow.clone(), items.to_vec(), event.clone());
        self.db
            .write(move |db| todo::workflows::create(db, &workflow, &items, &event))
            .await
    }
    async fn get_todo_workflow(&self, id: &str) -> Result<Option<TodoWorkflowRecord>> {
        let id = id.to_string();
        self.db.read(move |db| todo::workflows::get(db, &id)).await
    }
    async fn get_todo_workflow_detail(&self, id: &str) -> Result<Option<TodoWorkflowDetail>> {
        let id = id.to_string();
        self.db.read(move |db| todo::workflows::get_detail(db, &id)).await
    }
    async fn list_todo_workflows(&self, limit: u32) -> Result<Vec<TodoWorkflowSummary>> {
        self.db.read(move |db| todo::workflows::list(db, limit)).await
    }
    async fn list_todo_items(&self, workflow_id: &str) -> Result<Vec<TodoItemRecord>> {
        let wid = workflow_id.to_string();
        self.db.read(move |db| todo::items::items(db, &wid)).await
    }
    async fn list_todo_items_page(
        &self,
        workflow_id: &str,
        after_ordinal: Option<i64>,
        limit: u32,
    ) -> Result<crate::TodoItemPage> {
        let wid = workflow_id.to_string();
        self.db
            .read(move |db| todo::items::items_page(db, &wid, after_ordinal, limit))
            .await
    }
    async fn todo_workflow_field_chunk(
        &self,
        workflow_id: &str,
        field: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<PayloadChunkRecord>> {
        let (wid, field) = (workflow_id.to_string(), field.to_string());
        self.db
            .read(move |db| todo::workflows::field_chunk(db, &wid, &field, offset, max_bytes))
            .await
    }
    async fn todo_item_field_chunk(
        &self,
        workflow_id: &str,
        todo_id: &str,
        field: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<PayloadChunkRecord>> {
        let (wid, tid, field) = (workflow_id.to_string(), todo_id.to_string(), field.to_string());
        self.db
            .read(move |db| todo::items::field_chunk(db, &wid, &tid, &field, offset, max_bytes))
            .await
    }
    async fn commit_todo_transition(
        &self,
        workflow: &TodoWorkflowRecord,
        items: &[TodoItemRecord],
        event: &TodoEventRecord,
    ) -> Result<i64> {
        let (workflow, items, event) = (workflow.clone(), items.to_vec(), event.clone());
        self.db
            .write(move |db| todo::workflows::commit(db, &workflow, &items, &event))
            .await
    }
    async fn todo_events_after(
        &self,
        workflow_id: &str,
        after_seq: i64,
    ) -> Result<Vec<TodoEventRecord>> {
        let wid = workflow_id.to_string();
        self.db.read(move |db| todo::events::after(db, &wid, after_seq)).await
    }
    async fn todo_events_page(
        &self,
        workflow_id: &str,
        after_seq: i64,
        limit: u32,
        payload_budget: usize,
    ) -> Result<TodoEventPage> {
        let wid = workflow_id.to_string();
        self.db
            .read(move |db| todo::events::page(db, &wid, after_seq, limit, payload_budget, false))
            .await
    }
    async fn todo_events_before(
        &self,
        workflow_id: &str,
        before_seq: i64,
        limit: u32,
        payload_budget: usize,
    ) -> Result<TodoEventPage> {
        let wid = workflow_id.to_string();
        self.db
            .read(move |db| todo::events::page(db, &wid, before_seq, limit, payload_budget, true))
            .await
    }
    async fn last_todo_event_seq(&self, workflow_id: &str) -> Result<i64> {
        let wid = workflow_id.to_string();
        self.db.read(move |db| Ok(todo::events::last_seq(db, &wid))).await
    }
    async fn todo_event_payload_chunk(
        &self,
        workflow_id: &str,
        seq: i64,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<PayloadChunkRecord>> {
        let wid = workflow_id.to_string();
        self.db
            .read(move |db| todo::events::payload_chunk(db, &wid, seq, offset, max_bytes))
            .await
    }
        }
    };
}
