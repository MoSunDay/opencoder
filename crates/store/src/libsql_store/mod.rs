use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use libsql::Connection;
use tokio::sync::Mutex;

use crate::store::Store;
use crate::types::{
    ConvergedDagRun, DagDefRecord, DagEventRecord, DagRunRecord, Delivery, ImportReport,
    InputAdmission, MessageChunkPage, MessageRow, NodeRecord, NodeTaskRecord, NodeTaskStatus,
    SessionEventPage, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
    SessionMeta, SessionPatch, SubagentTaskRecord,
};
use crate::{
    BrainCapabilityDetail, BrainCapabilityRecord, BrainEngInputRecord, BrainPlanRecord,
    BrainVectorHit, BrainVectorWrite, TeamTopicRunRecord, TodoEventPage, TodoEventRecord,
    TodoItemRecord, TodoWorkflowDetail, TodoWorkflowRecord, TodoWorkflowSummary,
};

mod brain;
mod chat_tables;
mod connection;
mod dag;
mod dag_events;
mod events;
mod inputs;
mod messages;
mod node_state;
mod node_tasks;
mod nodes;
mod project;
mod project_runs;
pub(crate) mod schema;
mod sessions;
mod subagent_tasks;
mod team_runs;
mod todos;
mod tx;

/// Primary `Store` implementation backed by libsql (embedded local SQLite, WAL).
///
/// Holds ONE connection obtained at open time; each operation clones it. libsql
/// connection clones share the same underlying database — this makes in-memory
/// databases work correctly across operations (a fresh `db.connect()` per op
/// would hand back an empty `:memory:` db every time) while file-backed dbs
/// still get WAL semantics. All SQL lives in free functions in the submodules
/// so the backend can be swapped without touching callers.
pub struct LibsqlStore {
    conn: Connection,
    /// Serializes all DB operations. libsql 0.9.x local backend runs sync
    /// SQLite FFI directly on the tokio worker thread; without serialization,
    /// concurrent operations (multi-subagent flushers + run_loop) contend on
    /// SQLite's internal mutex, starving the runtime. An async Mutex yields on
    /// contention (never blocks a worker thread) while ensuring at most one
    /// worker touches SQLite FFI at a time.
    db_lock: Mutex<()>,
}

impl LibsqlStore {
    /// Open (or create) a libsql database file and bootstrap the schema.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            conn: connection::open_file(path.as_ref()).await?,
            db_lock: Mutex::new(()),
        })
    }

    /// Open an in-memory database (used by tests and ephemeral runs).
    pub async fn open_memory() -> Result<Self> {
        Ok(Self {
            conn: connection::open_memory().await?,
            db_lock: Mutex::new(()),
        })
    }

    /// Acquire a connection that shares the underlying database. Cheap clone.
    pub async fn conn(&self) -> Result<Connection> {
        Ok(self.conn.clone())
    }
}

#[async_trait]
impl Store for LibsqlStore {
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

    async fn save_brain_plan(&self, plan: &BrainPlanRecord) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        brain::save_plan(&self.conn, plan).await
    }

    async fn get_brain_plan(&self, id: &str) -> Result<Option<BrainPlanRecord>> {
        let _guard = self.db_lock.lock().await;
        brain::get_plan(&self.conn, id).await
    }

    async fn latest_brain_plan_for(&self, digest: &str) -> Result<Option<BrainPlanRecord>> {
        let _guard = self.db_lock.lock().await;
        brain::latest_plan_by_digest(&self.conn, digest).await
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
    async fn dispatch_node_task_for_session(
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
        node_tasks::dispatch_for_session(
            &self.conn, task_id, session_id, node_id, title, prompt, agent, model, now_ms,
        )
        .await
    }
    async fn claim_next_node_task(
        &self,
        node_id: &str,
        now_ms: i64,
    ) -> Result<Option<NodeTaskRecord>> {
        let _guard = self.db_lock.lock().await;
        node_tasks::claim_next(&self.conn, node_id, now_ms).await
    }
    async fn update_node_task_status(
        &self,
        task_id: &str,
        status: NodeTaskStatus,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        node_tasks::update_status(&self.conn, task_id, status, error, now_ms).await
    }
    async fn request_node_task_cancel(&self, task_id: &str) -> Result<Option<NodeTaskStatus>> {
        let _guard = self.db_lock.lock().await;
        node_tasks::request_cancel(&self.conn, task_id).await
    }
    async fn list_node_tasks(&self, node_id: &str, limit: u32) -> Result<Vec<NodeTaskRecord>> {
        let _guard = self.db_lock.lock().await;
        node_tasks::list_tasks(&self.conn, node_id, limit).await
    }
    async fn get_node_task(&self, task_id: &str) -> Result<Option<NodeTaskRecord>> {
        let _guard = self.db_lock.lock().await;
        node_tasks::get_task(&self.conn, task_id).await
    }
    async fn list_node_tasks_filtered(
        &self,
        node_id: Option<&str>,
        status: Option<NodeTaskStatus>,
        limit: u32,
    ) -> Result<Vec<NodeTaskRecord>> {
        let _guard = self.db_lock.lock().await;
        node_tasks::list_tasks_filtered(&self.conn, node_id, status, limit).await
    }
    async fn get_node_task_by_session(&self, session_id: &str) -> Result<Option<NodeTaskRecord>> {
        let _guard = self.db_lock.lock().await;
        node_tasks::get_by_session(&self.conn, session_id).await
    }
    async fn converge_lost_node_tasks(
        &self,
        now_ms: i64,
        stale_ms: i64,
    ) -> Result<Vec<NodeTaskRecord>> {
        let _guard = self.db_lock.lock().await;
        node_tasks::converge_lost(&self.conn, now_ms, stale_ms).await
    }

    async fn upsert_dag_def(&self, def: &DagDefRecord) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        dag::upsert_def(&self.conn, def).await
    }
    async fn list_dag_defs(&self) -> Result<Vec<DagDefRecord>> {
        let _guard = self.db_lock.lock().await;
        dag::list_defs(&self.conn).await
    }
    async fn get_dag_def(&self, id: &str) -> Result<Option<DagDefRecord>> {
        let _guard = self.db_lock.lock().await;
        dag::get_def(&self.conn, id).await
    }
    async fn delete_dag_def(&self, id: &str) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        dag::delete_def(&self.conn, id).await
    }
    async fn dispatch_dag_run(&self, run: &DagRunRecord) -> Result<DagRunRecord> {
        let _guard = self.db_lock.lock().await;
        dag::dispatch(&self.conn, run).await
    }
    async fn claim_next_dag_run(&self, node_id: &str, now_ms: i64) -> Result<Option<DagRunRecord>> {
        let _guard = self.db_lock.lock().await;
        dag::claim_next(&self.conn, node_id, now_ms).await
    }
    async fn update_dag_run_status(
        &self,
        run_id: &str,
        status: opencoder_dag::DagRunStatus,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<DagRunRecord> {
        let _guard = self.db_lock.lock().await;
        dag::update_status(&self.conn, run_id, status, error, now_ms).await
    }
    async fn finalize_dag_run(
        &self,
        run_id: &str,
        status: opencoder_dag::DagRunStatus,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<i64> {
        let _guard = self.db_lock.lock().await;
        dag::finalize_run(&self.conn, run_id, status, error, now_ms).await
    }
    async fn cancel_dag_run(&self, run_id: &str, now_ms: i64) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        dag::cancel(&self.conn, run_id, now_ms).await
    }
    async fn cancelling_dag_runs(&self, node_id: &str) -> Result<Vec<String>> {
        let _guard = self.db_lock.lock().await;
        dag::cancelling_runs(&self.conn, node_id).await
    }
    async fn get_dag_run(&self, id: &str) -> Result<Option<DagRunRecord>> {
        let _guard = self.db_lock.lock().await;
        dag::get_run(&self.conn, id).await
    }
    async fn list_dag_runs(&self, limit: u32) -> Result<Vec<DagRunRecord>> {
        let _guard = self.db_lock.lock().await;
        dag::list_runs(&self.conn, limit).await
    }
    async fn append_dag_events(&self, events: &[DagEventRecord]) -> Result<Vec<i64>> {
        let _guard = self.db_lock.lock().await;
        dag_events::append_events(&self.conn, events).await
    }
    async fn dag_events_after(
        &self,
        run_id: &str,
        after: i64,
        limit: u32,
    ) -> Result<Vec<DagEventRecord>> {
        let _guard = self.db_lock.lock().await;
        dag_events::events_after(&self.conn, run_id, after, limit).await
    }
    async fn converge_lost_dag_runs(
        &self,
        now_ms: i64,
        stale_ms: i64,
    ) -> Result<Vec<ConvergedDagRun>> {
        let _guard = self.db_lock.lock().await;
        dag::converge_lost(&self.conn, now_ms, stale_ms).await
    }

    // Team topic runs (opencoder-team fan-out ledger).
    async fn upsert_team_topic_run(&self, rec: &TeamTopicRunRecord) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        team_runs::upsert(&self.conn, rec).await
    }
    async fn finish_team_topic_run(&self, topic_id: &str) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        team_runs::finish(&self.conn, topic_id).await
    }
    async fn list_team_topic_runs(&self, topic_id: &str) -> Result<Vec<TeamTopicRunRecord>> {
        let _guard = self.db_lock.lock().await;
        team_runs::list(&self.conn, topic_id).await
    }

    async fn import_messages(
        &self,
        session_id: &str,
        msgs: &[opencoder_core::Message],
    ) -> Result<ImportReport> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        messages::import(&conn, session_id, msgs).await
    }
}
