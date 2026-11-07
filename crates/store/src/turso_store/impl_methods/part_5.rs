macro_rules! store_implementation_4 {
    ($($methods:tt)*) => {
        store_implementation_finish! {
            $($methods)*
    async fn finalize_dag_run(
        &self,
        run_id: &str,
        status: opencoder_dag::DagRunStatus,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<i64> {
        let run_id = run_id.to_owned();
        let error = error.map(ToOwned::to_owned);
        self.db.write(move |conn| async move {
            let run_id = run_id.as_str();
            let error = error.as_deref();

        dag::finalize_run(&conn, run_id, status, error, now_ms).await

        }).await
    }
    async fn cancel_dag_run(&self, run_id: &str, now_ms: i64) -> Result<()> {
        let run_id = run_id.to_owned();
        self.db.write(move |conn| async move {
            let run_id = run_id.as_str();

        dag::cancel(&conn, run_id, now_ms).await

        }).await
    }
    async fn cancelling_dag_runs(&self, node_id: &str) -> Result<Vec<String>> {
        let node_id = node_id.to_owned();
        self.db.read(move |conn| async move {
            let node_id = node_id.as_str();

        dag::cancelling_runs(&conn, node_id).await

        }).await
    }
    async fn get_dag_run(&self, id: &str) -> Result<Option<DagRunRecord>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        dag::get_run(&conn, id).await

        }).await
    }
    async fn list_dag_runs(&self, limit: u32) -> Result<Vec<DagRunRecord>> {
        self.db.read(move |conn| async move {

        dag::list_runs(&conn, limit).await

        }).await
    }
    async fn append_dag_events(&self, events: &[DagEventRecord]) -> Result<Vec<i64>> {
        let events = events.to_owned();
        self.db.write(move |conn| async move {
            let events = &events;

        dag_events::append_events(&conn, events).await

        }).await
    }
    async fn dag_events_after(
        &self,
        run_id: &str,
        after: i64,
        limit: u32,
    ) -> Result<Vec<DagEventRecord>> {
        let run_id = run_id.to_owned();
        self.db.read(move |conn| async move {
            let run_id = run_id.as_str();

        dag_events::events_after(&conn, run_id, after, limit).await

        }).await
    }
    async fn converge_lost_dag_runs(
        &self,
        now_ms: i64,
        stale_ms: i64,
    ) -> Result<Vec<ConvergedDagRun>> {
        self.db.write(move |conn| async move {

        dag::converge_lost(&conn, now_ms, stale_ms).await

        }).await
    }
    async fn upsert_team_topic_run(&self, rec: &TeamTopicRunRecord) -> Result<()> {
        let rec = rec.to_owned();
        self.db.write(move |conn| async move {
            let rec = &rec;

        team_runs::upsert(&conn, rec).await

        }).await
    }
    async fn finish_team_topic_run(&self, topic_id: &str) -> Result<()> {
        let topic_id = topic_id.to_owned();
        self.db.write(move |conn| async move {
            let topic_id = topic_id.as_str();

        team_runs::finish(&conn, topic_id).await

        }).await
    }
    async fn list_team_topic_runs(&self, topic_id: &str) -> Result<Vec<TeamTopicRunRecord>> {
        let topic_id = topic_id.to_owned();
        self.db.read(move |conn| async move {
            let topic_id = topic_id.as_str();

        team_runs::list(&conn, topic_id).await

        }).await
    }
    async fn record_schedule_run(&self, rec: &ScheduleRunRecord) -> Result<()> {
        let rec = rec.to_owned();
        self.db.write(move |conn| async move {
            let rec = &rec;

        schedule::record(&conn, rec).await

        }).await
    }
    async fn last_schedule_run(&self, schedule_id: &str) -> Result<Option<ScheduleRunRecord>> {
        let schedule_id = schedule_id.to_owned();
        self.db.read(move |conn| async move {
            let schedule_id = schedule_id.as_str();

        schedule::last(&conn, schedule_id).await

        }).await
    }
    async fn list_schedule_runs(
        &self,
        schedule_id: &str,
        limit: u32,
    ) -> Result<Vec<ScheduleRunRecord>> {
        let schedule_id = schedule_id.to_owned();
        self.db.read(move |conn| async move {
            let schedule_id = schedule_id.as_str();

        schedule::list(&conn, schedule_id, limit).await

        }).await
    }
    async fn upsert_schedule(
        &self,
        job: &opencoder_core::config::ScheduleJob,
        now_ms: i64,
    ) -> Result<()> {
        let job = job.to_owned();
        self.db.write(move |conn| async move {
            let job = &job;

        schedule::upsert_def(&conn, job, now_ms).await

        }).await
    }
    async fn get_schedule(&self, id: &str) -> Result<Option<ScheduleDefRecord>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        schedule::get_def(&conn, id).await

        }).await
    }
    async fn list_schedules(&self) -> Result<Vec<ScheduleDefRecord>> {
        self.db.read(move |conn| async move {

        schedule::list_defs(&conn).await

        }).await
    }
    async fn delete_schedule(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();

        schedule::delete_def(&conn, id).await

        }).await
    }
    async fn import_messages(
        &self,
        session_id: &str,
        msgs: &[opencoder_core::Message],
    ) -> Result<ImportReport> {
        let session_id = session_id.to_owned();
        let msgs = msgs.to_owned();
        self.db.write(move |conn| async move {
            let session_id = session_id.as_str();
            let msgs = &msgs;

        messages::import(&conn, session_id, msgs).await

        }).await
    }
        }
    };
}
