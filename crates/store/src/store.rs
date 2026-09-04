use anyhow::Result;
use async_trait::async_trait;

use opencoder_core::Message;

use crate::types::{
    ImportReport, MessageRow, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
    SessionMeta, SessionPatch, SubagentTaskRecord,
};
use crate::{
    BrainCapabilityDetail, BrainCapabilityRecord, BrainEngInputRecord, BrainVectorHit,
    BrainVectorWrite, TeamTopicRunRecord, TodoEventRecord, TodoItemRecord, TodoWorkflowRecord,
    TodoWorkflowSummary,
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
    /// Delete every session except `keep_session_id` (the currently-active
    /// one). Cascades to messages/inputs/events/subagent_tasks via the schema's
    /// `ON DELETE CASCADE` foreign keys. Returns the number of sessions removed.
    async fn clear_other_sessions(&self, keep_session_id: &str) -> Result<u64>;

    async fn append_message(&self, session_id: &str, msg: &Message) -> Result<i64>;
    async fn append_messages(&self, session_id: &str, msgs: &[Message]) -> Result<Vec<i64>>;
    async fn load_messages(&self, session_id: &str) -> Result<Vec<Message>>;
    /// Load messages for a session skipping the first `skip_count` persisted
    /// rows (ordered by insertion `seq` ASC), returning only the tail. Used by
    /// `resume` on the compaction path to avoid reloading the (potentially
    /// huge) soft-deleted compacted head. The default impl falls back to a full
    /// `load_messages` + in-memory drain so test fakes need not override it;
    /// the libsql backend overrides with an `OFFSET` query that skips without
    /// deserializing the dropped rows.
    async fn load_messages_after(&self, session_id: &str, skip_count: i64) -> Result<Vec<Message>> {
        let mut msgs = self.load_messages(session_id).await?;
        let skip = skip_count.clamp(0, i64::MAX) as usize;
        if skip < msgs.len() {
            msgs.drain(..skip);
        } else {
            msgs.clear();
        }
        Ok(msgs)
    }
    async fn last_message_seq(&self, session_id: &str) -> Result<i64>;

    /// Raw persisted message rows in `seq` order ([`MessageRow`] read model).
    /// Backs the P3 node message relay: the caller needs the true per-session
    /// `seq` (the resume boundary) plus the raw stored blocks, neither of
    /// which the decoded [`Message`] view carries. Default impl reconstructs
    /// from `load_messages` with positional seqs (1-based) so test fakes need
    /// not override it; the primary backend reads the real columns.
    async fn load_message_rows(&self, session_id: &str) -> Result<Vec<MessageRow>> {
        let msgs = self.load_messages(session_id).await?;
        Ok(msgs
            .into_iter()
            .enumerate()
            .map(|(i, m)| MessageRow {
                seq: i as i64 + 1,
                role: serde_json::to_value(m.role)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_else(|| "user".into()),
                blocks: serde_json::to_value(&m.blocks).unwrap_or(serde_json::Value::Null),
                created_at: m.created_at,
            })
            .collect())
    }

    async fn admit_input(&self, input: &SessionInput) -> Result<i64>;
    async fn pending_inputs(
        &self,
        session_id: &str,
        delivery: crate::types::Delivery,
    ) -> Result<Vec<SessionInput>>;
    async fn promote_inputs(
        &self,
        session_id: &str,
        up_to_admitted_seq: i64,
        delivery: crate::types::Delivery,
    ) -> Result<Vec<i64>>;
    async fn promote_next_queued(&self, session_id: &str) -> Result<Option<i64>>;
    /// Atomically return the oldest pending queued input (with its prompt) and
    /// mark it promoted. Used by the runner drain at idle to consume exactly one
    /// queued follow-up per cycle.
    async fn claim_next_queue(&self, session_id: &str) -> Result<Option<(i64, SessionInput)>>;
    /// Reset promoted inputs back to unpromoted (pending) state. Used by the
    /// runner's error-recovery path when a steer/queue batch fails
    /// mid-processing: items that were promoted but not yet consumed are
    /// restored so the next run picks them up. Idempotent — only touches rows
    /// that are currently promoted. Default no-op so test fakes need not
    /// override unless they exercise the promote/unpromote path.
    async fn unpromote_inputs(&self, _session_id: &str, _seqs: &[i64]) -> Result<()> {
        Ok(())
    }
    /// Mark promoted inputs as durably consumed (recorded into the transcript
    /// or applied as a control command). Idempotent. Best-effort callers may
    /// ignore errors: an unmarked row is recoverable by
    /// [`recover_orphan_inputs`]. Default no-op so test fakes keep compiling.
    async fn mark_inputs_recorded(&self, _session_id: &str, _seqs: &[i64]) -> Result<()> {
        Ok(())
    }
    /// Recover orphaned inputs (promoted but never recorded, e.g. after a
    /// crash or hard-cancel between promote and consume) back to pending so
    /// the next drain re-claims them. Idempotent; returns the number of
    /// recovered rows. Default no-op returning 0 so test fakes keep compiling.
    async fn recover_orphan_inputs(&self, _session_id: &str) -> Result<u64> {
        Ok(0)
    }
    /// Delete a pending input by its row id. Used by the TUI queue panel
    /// to let users remove a queued/steered prompt before it's consumed.
    async fn delete_input(&self, input_id: i64) -> Result<()>;
    /// Swap the drain order of two pending inputs by exchanging their
    /// `admitted_seq`. Both rows must belong to `session_id` and be still
    /// unpromoted. Used by the TUI queue panel to reorder follow-ups.
    async fn swap_input_order(&self, session_id: &str, seq_a: i64, seq_b: i64) -> Result<()>;

    /// Persist a batch of events atomically in a single transaction, returning
    /// the assigned `seq` for each event in input order. This is the preferred
    /// write path for high-frequency surfaces: one transaction (and thus one
    /// fsync at commit) replaces N single inserts. All events in a batch must
    /// share the same `session_id`.
    async fn append_events(&self, events: &[SessionEventRecord]) -> Result<Vec<i64>>;

    /// Persist a single event. Default impl delegates to [`append_events`].
    async fn append_event(&self, event: &SessionEventRecord) -> Result<i64> {
        let mut seqs = self.append_events(std::slice::from_ref(event)).await?;
        Ok(seqs.pop().unwrap_or(0))
    }
    async fn events_after(
        &self,
        session_id: &str,
        after_seq: i64,
    ) -> Result<Vec<SessionEventRecord>>;
    /// The highest persisted event seq for a session (0 if none). Used by a
    /// remote client to snapshot before posting a prompt so it only receives
    /// events generated by its own turn (mirrors `last_message_seq`).
    async fn last_event_seq(&self, session_id: &str) -> Result<i64>;

    /// Record a new subagent task (parent-child agent relationship) when a
    /// subagent is spawned. The task starts in `Running` status.
    async fn create_subagent_task(&self, record: &SubagentTaskRecord) -> Result<()>;
    /// Mark a subagent task as completed with its final result text and
    /// success/failure flag.
    async fn complete_subagent_task(&self, task_id: &str, result: &str, ok: bool) -> Result<()>;
    /// List all subagent tasks for a given parent session.
    async fn list_subagent_tasks(&self, parent_session_id: &str)
        -> Result<Vec<SubagentTaskRecord>>;
    /// Look up a single subagent task by its `task_id`. Returns `None` if no
    /// task matches. Used by `--session <task_id>` to resolve the parent
    /// session for resume.
    async fn get_subagent_task(&self, task_id: &str) -> Result<Option<SubagentTaskRecord>>;
    /// Mark a subagent task as cancelled (interrupted mid-run). Unlike
    /// `complete_subagent_task`, a cancelled task records no result -- its
    /// parent `task` tool_use stays open so the child can be replayed on the
    /// next user turn.
    async fn cancel_subagent_task(&self, task_id: &str) -> Result<()>;

    async fn create_todo_workflow(
        &self,
        _workflow: &TodoWorkflowRecord,
        _items: &[TodoItemRecord],
        _event: &TodoEventRecord,
    ) -> Result<i64> {
        anyhow::bail!(
            "todo workflows are not supported by {}",
            self.backend_name()
        )
    }
    async fn get_todo_workflow(&self, _id: &str) -> Result<Option<TodoWorkflowRecord>> {
        anyhow::bail!(
            "todo workflows are not supported by {}",
            self.backend_name()
        )
    }
    async fn list_todo_workflows(&self, _limit: u32) -> Result<Vec<TodoWorkflowSummary>> {
        anyhow::bail!(
            "todo workflows are not supported by {}",
            self.backend_name()
        )
    }
    async fn list_todo_items(&self, _workflow_id: &str) -> Result<Vec<TodoItemRecord>> {
        anyhow::bail!(
            "todo workflows are not supported by {}",
            self.backend_name()
        )
    }
    /// Atomically replace the workflow projection and its item projections,
    /// then append the transition event.
    async fn commit_todo_transition(
        &self,
        _workflow: &TodoWorkflowRecord,
        _items: &[TodoItemRecord],
        _event: &TodoEventRecord,
    ) -> Result<i64> {
        anyhow::bail!(
            "todo workflows are not supported by {}",
            self.backend_name()
        )
    }
    async fn todo_events_after(
        &self,
        _workflow_id: &str,
        _after_seq: i64,
    ) -> Result<Vec<TodoEventRecord>> {
        anyhow::bail!(
            "todo workflows are not supported by {}",
            self.backend_name()
        )
    }

    /// Persist a new brain capability together with its exemplar inputs in
    /// one transaction (project goals / capability library). Step-wise write:
    /// no embedding row is touched, so runtime paths must prefer
    /// [`create_brain_capability_with_vector`], which commits capability +
    /// eng_inputs + vector atomically; this variant stays for direct
    /// store-level tooling/tests.
    async fn create_brain_capability(
        &self,
        _capability: &BrainCapabilityRecord,
        _eng_inputs: &[BrainEngInputRecord],
    ) -> Result<()> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Update every capability field and replace its exemplar inputs
    /// atomically. Step-wise write: the embedding row is left untouched, so
    /// runtime paths must prefer [`update_brain_capability_with_vector`],
    /// which swaps content + eng_inputs + vector in one transaction (no
    /// stale-vector window); this variant stays for direct store-level
    /// tooling/tests.
    async fn update_brain_capability(
        &self,
        _capability: &BrainCapabilityRecord,
        _eng_inputs: &[BrainEngInputRecord],
    ) -> Result<()> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Delete a capability; exemplar inputs and vectors cascade.
    async fn delete_brain_capability(&self, _id: &str) -> Result<()> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Fetch one capability with its ordered exemplar inputs.
    async fn get_brain_capability(&self, _id: &str) -> Result<Option<BrainCapabilityDetail>> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Fetch every capability (newest first) with its exemplar inputs.
    async fn list_brain_capabilities(&self) -> Result<Vec<BrainCapabilityDetail>> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Insert-or-replace the embedding for a capability. `emb` is the
    /// little-endian f32 byte encoding shared with vector search.
    async fn upsert_brain_vector(
        &self,
        _capability_id: &str,
        _dim: i64,
        _model: &str,
        _emb: &[u8],
        _updated_at: i64,
    ) -> Result<()> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Create a capability, replace its exemplar inputs and upsert its
    /// embedding in ONE transaction. The vector is embedded by the caller
    /// (brain runtime) beforehand and passed in as raw bytes, eliminating the
    /// cross-table partial-write window where a capability row is persisted
    /// but its vector is missing.
    async fn create_brain_capability_with_vector(
        &self,
        _capability: &BrainCapabilityRecord,
        _eng_inputs: &[BrainEngInputRecord],
        _vector: &BrainVectorWrite,
    ) -> Result<()> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Update every capability field, replace its exemplar inputs and
    /// INSERT OR REPLACE its embedding in ONE transaction. The vector is
    /// embedded by the caller (brain runtime) beforehand and passed in as
    /// raw bytes, eliminating the cross-table partial-write window where the
    /// capability content is new but a stale old vector still answers search.
    async fn update_brain_capability_with_vector(
        &self,
        _capability: &BrainCapabilityRecord,
        _eng_inputs: &[BrainEngInputRecord],
        _vector: &BrainVectorWrite,
    ) -> Result<()> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Nearest-neighbour cosine-distance search over stored embeddings,
    /// scoped to one embedding model, ascending by distance.
    async fn search_brain_vectors(
        &self,
        _model: &str,
        _query_emb: &[u8],
        _limit: u32,
    ) -> Result<Vec<BrainVectorHit>> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }

    /// Register (or re-register) a worker node by its unique `name`. A new
    /// name gets a fresh ULID; a known name keeps its `id` so dispatched tasks
    /// keep their foreign key, while version/workdir/last_seen_at are
    /// refreshed and `last_status` resets to `online`.
    async fn register_node(
        &self,
        _name: &str,
        _version: Option<&str>,
        _workdir: Option<&str>,
        _addr: Option<&str>,
        _now_ms: i64,
    ) -> Result<crate::types::NodeRecord> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    async fn list_nodes(&self) -> Result<Vec<crate::types::NodeRecord>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    async fn get_node(&self, _id: &str) -> Result<Option<crate::types::NodeRecord>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Delete a worker node; cascades to its node_tasks and from there to each
    /// task's synthetic session (`ON DELETE CASCADE` chain in the schema).
    async fn delete_node(&self, _id: &str) -> Result<()> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Liveness touch + cancel-command poll in one transaction: refreshes
    /// `last_seen_at`, collapses non-busy status to `idle`, and returns the
    /// ids of this node's cancelling tasks as the cancel instructions.
    async fn heartbeat_node(&self, _id: &str, _now_ms: i64) -> Result<Vec<String>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Enqueue a node task plus its synthetic session (`task_type == "node"`)
    /// atomically. The task starts `pending` and stays queued until the node
    /// claims it via [`Store::claim_next_node_task`].
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_node_task(
        &self,
        _task_id: &str,
        _session_id: &str,
        _node_id: &str,
        _title: Option<&str>,
        _prompt: &str,
        _agent: Option<&str>,
        _model: Option<&str>,
        _now_ms: i64,
    ) -> Result<crate::types::NodeTaskRecord> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Enqueue a node task bound to an EXISTING session (the console's
    /// "continue this dialog" flow): only the `node_tasks` row is created, the
    /// session row is reused as-is. Errors when the session does not exist so
    /// the HTTP layer can answer 400 instead of dangling the FK.
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_node_task_for_session(
        &self,
        _task_id: &str,
        _session_id: &str,
        _node_id: &str,
        _title: Option<&str>,
        _prompt: &str,
        _agent: Option<&str>,
        _model: Option<&str>,
        _now_ms: i64,
    ) -> Result<crate::types::NodeTaskRecord> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Atomically claim the oldest pending task of `node_id` (FIFO, CAS-guarded
    /// so concurrent claimers never double-dispatch). Returns `None` when the
    /// node already runs a task (single-active-task policy) or nothing is due.
    async fn claim_next_node_task(
        &self,
        _node_id: &str,
        _now_ms: i64,
    ) -> Result<Option<crate::types::NodeTaskRecord>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Move a node task along its state machine (`pending/running/cancelling`
    /// toward `done|error|cancelled`). Illegal transitions error out; terminal
    /// writes stamp `finished_at` and release the node's busy slot.
    async fn update_node_task_status(
        &self,
        _task_id: &str,
        _status: crate::types::NodeTaskStatus,
        _error: Option<&str>,
        _now_ms: i64,
    ) -> Result<()> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Request cancellation of a pending/running node task. Returns the
    /// pre-cancel status (`Pending` = queue removal is enough, `Running` =
    /// the node must observe it on heartbeat); `None` for already-cancelling /
    /// terminal / unknown tasks.
    async fn request_node_task_cancel(
        &self,
        _task_id: &str,
    ) -> Result<Option<crate::types::NodeTaskStatus>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    async fn list_node_tasks(
        &self,
        _node_id: &str,
        _limit: u32,
    ) -> Result<Vec<crate::types::NodeTaskRecord>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    async fn get_node_task(&self, _task_id: &str) -> Result<Option<crate::types::NodeTaskRecord>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Fleet-wide task listing with optional `node_id` / `status` filters.
    /// FIFO order (`created_at ASC, rowid ASC`) — the exact order a node's
    /// claim loop drains in, with the same-ms `rowid` tiebreak.
    async fn list_node_tasks_filtered(
        &self,
        _node_id: Option<&str>,
        _status: Option<crate::types::NodeTaskStatus>,
        _limit: u32,
    ) -> Result<Vec<crate::types::NodeTaskRecord>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Reverse lookup: the node task owning a synthetic session (`None` for
    /// ordinary sessions — not an error).
    async fn get_node_task_by_session(
        &self,
        _session_id: &str,
    ) -> Result<Option<crate::types::NodeTaskRecord>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Collapse zombie tasks of nodes whose latest heartbeat is older than
    /// `stale_ms`: any `running | cancelling` task of such a node becomes
    /// `error("node lost")` (terminal-frozen). Returns the converged records.
    async fn converge_lost_node_tasks(
        &self,
        _now_ms: i64,
        _stale_ms: i64,
    ) -> Result<Vec<crate::types::NodeTaskRecord>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }

    // ---------------- DAG workflow store API (node-side scheduling) ----------
    //
    // The server stores defs/runs/events and runs the SAME claim /
    // cancel-piggyback / lost-sweep protocols as node_tasks; the node
    // executes the whole workflow. See `opencoder_dag` for the domain.

    /// Upsert a DAG definition by `spec.name` (id stays stable across edits).
    async fn upsert_dag_def(&self, _def: &crate::types::DagDefRecord) -> Result<()> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }
    async fn list_dag_defs(&self) -> Result<Vec<crate::types::DagDefRecord>> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }
    async fn get_dag_def(&self, _id: &str) -> Result<Option<crate::types::DagDefRecord>> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }
    async fn delete_dag_def(&self, _id: &str) -> Result<()> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }
    /// Enqueue a run: validates nothing (the web layer validates the spec),
    /// snapshots `spec_json`, and inserts `pending`. A pinned `node_id`
    /// restricts claiming to that node; `None` means any node.
    async fn dispatch_dag_run(
        &self,
        _run: &crate::types::DagRunRecord,
    ) -> Result<crate::types::DagRunRecord> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }
    /// FIFO claim of the oldest pending run this `node_id` may take
    /// (pinned-to-node or unpinned), CAS-guarded inside `BEGIN IMMEDIATE`;
    /// `None` when the node already runs a DAG run or nothing is due.
    async fn claim_next_dag_run(
        &self,
        _node_id: &str,
        _now_ms: i64,
    ) -> Result<Option<crate::types::DagRunRecord>> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }
    /// Terminal or intermediate status move along the run state machine
    /// (`transition_allowed` grid); terminal writes stamp `finished_at`.
    async fn update_dag_run_status(
        &self,
        _run_id: &str,
        _status: opencoder_dag::DagRunStatus,
        _error: Option<&str>,
        _now_ms: i64,
    ) -> Result<crate::types::DagRunRecord> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }
    /// Mark a run `cancelling` (or `cancelled` straight from `pending`);
    /// the node observes it via the heartbeat piggyback and aborts.
    async fn cancel_dag_run(&self, _run_id: &str, _now_ms: i64) -> Result<()> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }
    /// `cancelling` runs of `node_id` — the heartbeat piggyback payload.
    async fn cancelling_dag_runs(&self, _node_id: &str) -> Result<Vec<String>> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }
    async fn get_dag_run(&self, _id: &str) -> Result<Option<crate::types::DagRunRecord>> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }
    async fn list_dag_runs(&self, _limit: u32) -> Result<Vec<crate::types::DagRunRecord>> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }
    /// Append node-uploaded events (append-only; assigns `seq`).
    async fn append_dag_events(
        &self,
        _events: &[crate::types::DagEventRecord],
    ) -> Result<Vec<i64>> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }
    /// Replay slice for the run SSE (`seq > after`, ascending).
    async fn dag_events_after(
        &self,
        _run_id: &str,
        _after: i64,
        _limit: u32,
    ) -> Result<Vec<crate::types::DagEventRecord>> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }
    /// Lost-node sweep for DAG runs — same semantics as
    /// [`Store::converge_lost_node_tasks`]: `running | cancelling` runs of
    /// heartbeat-stale nodes become `error("node lost")`.
    async fn converge_lost_dag_runs(
        &self,
        _now_ms: i64,
        _stale_ms: i64,
    ) -> Result<Vec<crate::types::DagRunRecord>> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }

    // ------------- Team topic runs (opencode-team fan-out) -----------------
    //
    // The durable (topic, node) pairing ledger of the multi-node team
    // runtime. Pure persistence: scheduling lives above the Store.

    /// Insert or refresh one `(topic_id, node_id)` run row. When the row
    /// already exists only `status` moves — `created_at` keeps its original
    /// value, so refreshing a pairing never restarts the run's clock.
    async fn upsert_team_topic_run(&self, _rec: &TeamTopicRunRecord) -> Result<()> {
        anyhow::bail!("team store API is not supported by {}", self.backend_name())
    }
    /// Flip EVERY row of `topic_id` to `finished` (the topic is done; nodes
    /// still executing converge on their next write). No-op for unknown topics.
    async fn finish_team_topic_run(&self, _topic_id: &str) -> Result<()> {
        anyhow::bail!("team store API is not supported by {}", self.backend_name())
    }
    /// All run rows of `topic_id`, oldest `created_at` first.
    async fn list_team_topic_runs(&self, _topic_id: &str) -> Result<Vec<TeamTopicRunRecord>> {
        anyhow::bail!("team store API is not supported by {}", self.backend_name())
    }

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
