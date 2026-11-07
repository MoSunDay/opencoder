// Method group: execution ledger — node-task queue, DAG definitions/runs/
// events, team topic runs, schedules and session deletes. Same adapter
// shape as part_1: clone args, run the domain free-function under the
// engine's write lease (mutations) or the lease-free read path (queries).
use crate::rdb_store::execution as ex;
use crate::schedule_types::ScheduleRunRecord;
use crate::team_types::TeamTopicRunRecord;
use crate::types::{ClearNodeDialogs, ConvergedDagRun, DagDefRecord, DagRunRecord};
use crate::types::{NodeTaskRecord, NodeTaskStatus};
use opencoder_dag::DagRunStatus;

macro_rules! rdb_impl_2 {
    ($($methods:tt)*) => {
        rdb_impl_3! {
            $($methods)*

    // ------------- Node task queue ------------------------------------------

    #[allow(clippy::too_many_arguments)]
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
        let (task_id, session_id, node_id) =
            (task_id.to_string(), session_id.to_string(), node_id.to_string());
        let (title, prompt, agent, model) = (
            title.map(str::to_string),
            prompt.to_string(),
            agent.map(str::to_string),
            model.map(str::to_string),
        );
        self.db
            .write(move |db| {
                ex::node_tasks::dispatch(
                    db,
                    &task_id,
                    &session_id,
                    &node_id,
                    title.as_deref(),
                    &prompt,
                    agent.as_deref(),
                    model.as_deref(),
                    now_ms,
                )
            })
            .await
    }
    #[allow(clippy::too_many_arguments)]
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
        let (task_id, session_id, node_id) =
            (task_id.to_string(), session_id.to_string(), node_id.to_string());
        let (title, prompt, agent, model) = (
            title.map(str::to_string),
            prompt.to_string(),
            agent.map(str::to_string),
            model.map(str::to_string),
        );
        self.db
            .write(move |db| {
                ex::node_tasks::dispatch_for_session(
                    db,
                    &task_id,
                    &session_id,
                    &node_id,
                    title.as_deref(),
                    &prompt,
                    agent.as_deref(),
                    model.as_deref(),
                    now_ms,
                )
            })
            .await
    }
    async fn claim_next_node_task(
        &self,
        node_id: &str,
        now_ms: i64,
    ) -> Result<Option<NodeTaskRecord>> {
        let node_id = node_id.to_string();
        self.db
            .write(move |db| ex::node_tasks::claim_next(db, &node_id, now_ms))
            .await
    }
    async fn update_node_task_status(
        &self,
        task_id: &str,
        status: NodeTaskStatus,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<()> {
        let (task_id, error) = (task_id.to_string(), error.map(str::to_string));
        self.db
            .write(move |db| {
                ex::node_tasks::update_status(db, &task_id, status, error.as_deref(), now_ms)
            })
            .await
    }
    async fn request_node_task_cancel(
        &self,
        task_id: &str,
    ) -> Result<Option<NodeTaskStatus>> {
        let task_id = task_id.to_string();
        self.db
            .write(move |db| ex::node_tasks::request_cancel(db, &task_id))
            .await
    }
    async fn list_node_tasks(
        &self,
        node_id: &str,
        limit: u32,
    ) -> Result<Vec<NodeTaskRecord>> {
        let node_id = node_id.to_string();
        self.db
            .read(move |db| ex::node_tasks::list_tasks(db, &node_id, limit))
            .await
    }
    async fn get_node_task(&self, task_id: &str) -> Result<Option<NodeTaskRecord>> {
        let task_id = task_id.to_string();
        self.db
            .read(move |db| ex::node_tasks::get_task(db, &task_id))
            .await
    }
    async fn list_node_tasks_filtered(
        &self,
        node_id: Option<&str>,
        status: Option<NodeTaskStatus>,
        limit: u32,
    ) -> Result<Vec<NodeTaskRecord>> {
        let node_id = node_id.map(str::to_string);
        self.db
            .read(move |db| {
                ex::node_tasks::list_tasks_filtered(db, node_id.as_deref(), status, limit)
            })
            .await
    }
    async fn get_node_task_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<NodeTaskRecord>> {
        let session_id = session_id.to_string();
        self.db
            .read(move |db| ex::node_tasks::get_by_session(db, &session_id))
            .await
    }
    async fn converge_lost_node_tasks(
        &self,
        now_ms: i64,
        stale_ms: i64,
    ) -> Result<Vec<NodeTaskRecord>> {
        self.db
            .write(move |db| ex::node_tasks::converge_lost(db, now_ms, stale_ms))
            .await
    }
    async fn clear_node_dialogs(&self, node_id: &str) -> Result<ClearNodeDialogs> {
        let node_id = node_id.to_string();
        self.db
            .write(move |db| ex::node_tasks::clear_finished_sessions(db, &node_id))
            .await
    }
    async fn delete_sessions(&self, ids: &[String]) -> Result<u64> {
        let ids = ids.to_vec();
        self.db.write(move |db| ex::node_tasks::delete_sessions(db, &ids)).await
    }

    // ------------- DAG definitions & runs ------------------------------------

    async fn upsert_dag_def(&self, def: &DagDefRecord) -> Result<()> {
        let def = def.clone();
        self.db.write(move |db| ex::dag::upsert_def(db, &def)).await
    }
    async fn list_dag_defs(&self) -> Result<Vec<DagDefRecord>> {
        self.db.read(move |db| ex::dag::list_defs(db)).await
    }
    async fn get_dag_def(&self, id: &str) -> Result<Option<DagDefRecord>> {
        let id = id.to_string();
        self.db.read(move |db| ex::dag::get_def(db, &id)).await
    }
    async fn delete_dag_def(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.db.write(move |db| ex::dag::delete_def(db, &id)).await
    }
    async fn dispatch_dag_run(&self, run: &DagRunRecord) -> Result<DagRunRecord> {
        let run = run.clone();
        self.db.write(move |db| ex::dag::dispatch(db, &run)).await
    }
    async fn claim_next_dag_run(
        &self,
        node_id: &str,
        now_ms: i64,
    ) -> Result<Option<DagRunRecord>> {
        let node_id = node_id.to_string();
        self.db
            .write(move |db| ex::dag::claim_next(db, &node_id, now_ms))
            .await
    }
    async fn update_dag_run_status(
        &self,
        run_id: &str,
        status: DagRunStatus,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<DagRunRecord> {
        let (run_id, error) = (run_id.to_string(), error.map(str::to_string));
        self.db
            .write(move |db| {
                ex::dag::update_status(db, &run_id, status, error.as_deref(), now_ms)
            })
            .await
    }
    async fn finalize_dag_run(
        &self,
        run_id: &str,
        status: DagRunStatus,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<i64> {
        let (run_id, error) = (run_id.to_string(), error.map(str::to_string));
        self.db
            .write(move |db| {
                ex::dag::finalize_run(db, &run_id, status, error.as_deref(), now_ms)
            })
            .await
    }
    async fn cancel_dag_run(&self, run_id: &str, now_ms: i64) -> Result<()> {
        let run_id = run_id.to_string();
        self.db.write(move |db| ex::dag::cancel(db, &run_id, now_ms)).await
    }
    async fn cancelling_dag_runs(&self, node_id: &str) -> Result<Vec<String>> {
        let node_id = node_id.to_string();
        self.db
            .read(move |db| ex::dag::cancelling_runs(db, &node_id))
            .await
    }
    async fn get_dag_run(&self, id: &str) -> Result<Option<DagRunRecord>> {
        let id = id.to_string();
        self.db.read(move |db| ex::dag::get_run(db, &id)).await
    }
    async fn list_dag_runs(&self, limit: u32) -> Result<Vec<DagRunRecord>> {
        self.db.read(move |db| ex::dag::list_runs(db, limit)).await
    }
    async fn append_dag_events(
        &self,
        events: &[crate::types::DagEventRecord],
    ) -> Result<Vec<i64>> {
        let events = events.to_vec();
        self.db
            .write(move |db| ex::dag_events::append_events(db, &events))
            .await
    }
    async fn dag_events_after(
        &self,
        run_id: &str,
        after: i64,
        limit: u32,
    ) -> Result<Vec<crate::types::DagEventRecord>> {
        let run_id = run_id.to_string();
        self.db
            .read(move |db| ex::dag_events::events_after(db, &run_id, after, limit))
            .await
    }
    async fn converge_lost_dag_runs(
        &self,
        now_ms: i64,
        stale_ms: i64,
    ) -> Result<Vec<ConvergedDagRun>> {
        self.db
            .write(move |db| ex::dag::converge_lost(db, now_ms, stale_ms))
            .await
    }

    // ------------- Team topic runs -------------------------------------------

    async fn upsert_team_topic_run(&self, rec: &TeamTopicRunRecord) -> Result<()> {
        let rec = rec.clone();
        self.db.write(move |db| ex::team_runs::upsert(db, &rec)).await
    }
    async fn finish_team_topic_run(&self, topic_id: &str) -> Result<()> {
        let topic_id = topic_id.to_string();
        self.db
            .write(move |db| ex::team_runs::finish(db, &topic_id))
            .await
    }
    async fn list_team_topic_runs(&self, topic_id: &str) -> Result<Vec<TeamTopicRunRecord>> {
        let topic_id = topic_id.to_string();
        self.db
            .read(move |db| ex::team_runs::list(db, &topic_id))
            .await
    }

    // ------------- Schedules --------------------------------------------------

    async fn record_schedule_run(&self, rec: &ScheduleRunRecord) -> Result<()> {
        let rec = rec.clone();
        self.db.write(move |db| ex::schedule::record(db, &rec)).await
    }
    async fn last_schedule_run(&self, schedule_id: &str) -> Result<Option<ScheduleRunRecord>> {
        let schedule_id = schedule_id.to_string();
        self.db
            .read(move |db| ex::schedule::last(db, &schedule_id))
            .await
    }
    async fn list_schedule_runs(
        &self,
        schedule_id: &str,
        limit: u32,
    ) -> Result<Vec<ScheduleRunRecord>> {
        let schedule_id = schedule_id.to_string();
        self.db
            .read(move |db| ex::schedule::list(db, &schedule_id, limit))
            .await
    }
    async fn upsert_schedule(
        &self,
        job: &opencoder_core::config::ScheduleJob,
        now_ms: i64,
    ) -> Result<()> {
        let job = job.clone();
        self.db
            .write(move |db| ex::schedule::upsert_def(db, &job, now_ms))
            .await
    }
    async fn get_schedule(
        &self,
        id: &str,
    ) -> Result<Option<crate::schedule_types::ScheduleDefRecord>> {
        let id = id.to_string();
        self.db.read(move |db| ex::schedule::get_def(db, &id)).await
    }
    async fn list_schedules(&self) -> Result<Vec<crate::schedule_types::ScheduleDefRecord>> {
        self.db.read(move |db| ex::schedule::list_defs(db)).await
    }
    async fn delete_schedule(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.db.write(move |db| ex::schedule::delete_def(db, &id)).await
    }
        }
    };
}
