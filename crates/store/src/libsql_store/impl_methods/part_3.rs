// Method group 3; assembled before async_trait expands the complete contract.
macro_rules! store_implementation_2 {
    ($($methods:tt)*) => {
        store_implementation_finish! {
            $($methods)*
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

    async fn clear_node_dialogs(&self, node_id: &str) -> Result<ClearNodeDialogs> {
        let _guard = self.db_lock.lock().await;
        node_tasks::clear_finished_sessions(&self.conn, node_id).await
    }

    async fn delete_sessions(&self, ids: &[String]) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        sessions::delete_many(&conn, ids).await
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

    // Schedule runs (control-plane cron fire ledger).
    async fn record_schedule_run(&self, rec: &ScheduleRunRecord) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        schedule::record(&self.conn, rec).await
    }
    async fn last_schedule_run(&self, schedule_id: &str) -> Result<Option<ScheduleRunRecord>> {
        let _guard = self.db_lock.lock().await;
        schedule::last(&self.conn, schedule_id).await
    }
    async fn list_schedule_runs(
        &self,
        schedule_id: &str,
        limit: u32,
    ) -> Result<Vec<ScheduleRunRecord>> {
        let _guard = self.db_lock.lock().await;
        schedule::list(&self.conn, schedule_id, limit).await
    }

    // Schedule definitions (control-plane cron jobs, schema v27).
    async fn upsert_schedule(
        &self,
        job: &opencoder_core::config::ScheduleJob,
        now_ms: i64,
    ) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        schedule::upsert_def(&self.conn, job, now_ms).await
    }
    async fn get_schedule(&self, id: &str) -> Result<Option<ScheduleDefRecord>> {
        let _guard = self.db_lock.lock().await;
        schedule::get_def(&self.conn, id).await
    }
    async fn list_schedules(&self) -> Result<Vec<ScheduleDefRecord>> {
        let _guard = self.db_lock.lock().await;
        schedule::list_defs(&self.conn).await
    }
    async fn delete_schedule(&self, id: &str) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        schedule::delete_def(&self.conn, id).await
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
    };
}
