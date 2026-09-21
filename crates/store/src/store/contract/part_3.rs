// Method group 3; assembled before async_trait expands the complete contract.
macro_rules! store_contract_2 {
    ($($methods:tt)*) => {
        store_contract_finish! {
            $($methods)*
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

    /// Bulk-clear a node's console dialogs: delete every session bound to a
    /// TERMINAL node task of `node_id` (done | error | cancelled) — the FK
    /// cascades take the node_tasks row plus messages/inputs/events/subagent
    /// tasks with it. Sessions whose node task is still pending/running/
    /// cancelling are KEPT so a running execution survives the sweep.
    /// Returns how many sessions were removed and which ids were skipped.
    async fn clear_node_dialogs(&self, _node_id: &str) -> Result<crate::types::ClearNodeDialogs> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }

    /// Delete sessions by id in one batch (child rows — messages, inputs,
    /// events, subagent_tasks, node_tasks — cascade via foreign keys).
    /// Unknown ids are ignored. Returns the number of deleted session rows.
    async fn delete_sessions(&self, _ids: &[String]) -> Result<u64> {
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
    /// Terminal status move + synthetic `run_finished` event in ONE
    /// transaction; returns the event seq. Same error contract as
    /// [`Store::update_dag_run_status`] ("not found" / "illegal").
    async fn finalize_dag_run(
        &self,
        _run_id: &str,
        _status: opencoder_dag::DagRunStatus,
        _error: Option<&str>,
        _now_ms: i64,
    ) -> Result<i64> {
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
    /// heartbeat-stale nodes become `error("node lost")`; each converged run
    /// carries its in-transaction `run_finished` seq.
    async fn converge_lost_dag_runs(
        &self,
        _now_ms: i64,
        _stale_ms: i64,
    ) -> Result<Vec<crate::types::ConvergedDagRun>> {
        anyhow::bail!("dag store API is not supported by {}", self.backend_name())
    }

    // ------------- Team topic runs (opencoder-team fan-out) -----------------
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

    // ------------- Schedule runs (control-plane cron ledger) ---------------
    //
    // The fire history of `schedules.json` jobs, persisted by the control
    // plane's cron scheduler. Pure persistence: scheduling lives above the
    // Store.

    /// Insert or replace one fire row (keyed `(schedule_id, scheduled_for_ms)`,
    /// so an error-retry of the same tick converges instead of duplicating).
    async fn record_schedule_run(&self, _rec: &ScheduleRunRecord) -> Result<()> {
        anyhow::bail!(
            "schedule store API is not supported by {}",
            self.backend_name()
        )
    }
    /// The most recent row of `schedule_id`, or `None` before its first fire.
    async fn last_schedule_run(&self, _schedule_id: &str) -> Result<Option<ScheduleRunRecord>> {
        anyhow::bail!(
            "schedule store API is not supported by {}",
            self.backend_name()
        )
    }
    /// History of `schedule_id`, newest tick first, at most `limit` rows.
    async fn list_schedule_runs(
        &self,
        _schedule_id: &str,
        _limit: u32,
    ) -> Result<Vec<ScheduleRunRecord>> {
        anyhow::bail!(
            "schedule store API is not supported by {}",
            self.backend_name()
        )
    }

    // ------------- Schedule definitions (control-plane cron jobs) ----------
    //
    // Since schema v27 the `schedules` table IS the scheduler's definition
    // source of truth; `schedules.json` is only a bootstrap seed. CRUD is
    // admin-only at the HTTP layer; the Store stays policy-free.

    /// Insert or update one definition by id. `created_at` is stamped on
    /// first insert and preserved across updates; `updated_at` moves to
    /// `now_ms`. No validation here — callers validate
    /// (`ScheduleJob::validate`) before calling.
    async fn upsert_schedule(
        &self,
        _job: &opencoder_core::config::ScheduleJob,
        _now_ms: i64,
    ) -> Result<()> {
        anyhow::bail!(
            "schedule store API is not supported by {}",
            self.backend_name()
        )
    }
    /// One definition (with timestamps) by id, or `None`.
    async fn get_schedule(
        &self,
        _id: &str,
    ) -> Result<Option<crate::schedule_types::ScheduleDefRecord>> {
        anyhow::bail!(
            "schedule store API is not supported by {}",
            self.backend_name()
        )
    }
    /// Every definition, stable id order.
    async fn list_schedules(&self) -> Result<Vec<crate::schedule_types::ScheduleDefRecord>> {
        anyhow::bail!(
            "schedule store API is not supported by {}",
            self.backend_name()
        )
    }
    /// Delete one definition. Fire history (`schedule_runs`) is kept — the
    /// ledger is an audit trail and has no FK by design.
    async fn delete_schedule(&self, _id: &str) -> Result<()> {
        anyhow::bail!(
            "schedule store API is not supported by {}",
            self.backend_name()
        )
    }

    async fn import_messages(&self, session_id: &str, msgs: &[Message]) -> Result<ImportReport> {
        let seqs = self.append_messages(session_id, msgs).await?;
        Ok(message_projection::import_report(seqs.len()))
    }

        }
    };
}
