macro_rules! store_implementation_3 {
    ($($methods:tt)*) => {
        store_implementation_4! {
            $($methods)*
    async fn register_node(
        &self,
        name: &str,
        version: Option<&str>,
        workdir: Option<&str>,
        addr: Option<&str>,
        now_ms: i64,
    ) -> Result<NodeRecord> {
        let name = name.to_owned();
        let version = version.map(ToOwned::to_owned);
        let workdir = workdir.map(ToOwned::to_owned);
        let addr = addr.map(ToOwned::to_owned);
        self.db.write(move |conn| async move {
            let name = name.as_str();
            let version = version.as_deref();
            let workdir = workdir.as_deref();
            let addr = addr.as_deref();

        nodes::register(&conn, name, version, workdir, addr, now_ms).await

        }).await
    }
    async fn list_nodes(&self) -> Result<Vec<NodeRecord>> {
        self.db.read(move |conn| async move {

        nodes::list(&conn).await

        }).await
    }
    async fn get_node(&self, id: &str) -> Result<Option<NodeRecord>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        nodes::get(&conn, id).await

        }).await
    }
    async fn delete_node(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();

        nodes::delete(&conn, id).await

        }).await
    }
    async fn heartbeat_node(&self, id: &str, now_ms: i64) -> Result<Vec<String>> {
        let id = id.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();

        nodes::heartbeat(&conn, id, now_ms).await

        }).await
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
        let task_id = task_id.to_owned();
        let session_id = session_id.to_owned();
        let node_id = node_id.to_owned();
        let title = title.map(ToOwned::to_owned);
        let prompt = prompt.to_owned();
        let agent = agent.map(ToOwned::to_owned);
        let model = model.map(ToOwned::to_owned);
        self.db.write(move |conn| async move {
            let task_id = task_id.as_str();
            let session_id = session_id.as_str();
            let node_id = node_id.as_str();
            let title = title.as_deref();
            let prompt = prompt.as_str();
            let agent = agent.as_deref();
            let model = model.as_deref();

        node_tasks::dispatch(
            &conn, task_id, session_id, node_id, title, prompt, agent, model, now_ms,
        )
        .await

        }).await
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
        let task_id = task_id.to_owned();
        let session_id = session_id.to_owned();
        let node_id = node_id.to_owned();
        let title = title.map(ToOwned::to_owned);
        let prompt = prompt.to_owned();
        let agent = agent.map(ToOwned::to_owned);
        let model = model.map(ToOwned::to_owned);
        self.db.write(move |conn| async move {
            let task_id = task_id.as_str();
            let session_id = session_id.as_str();
            let node_id = node_id.as_str();
            let title = title.as_deref();
            let prompt = prompt.as_str();
            let agent = agent.as_deref();
            let model = model.as_deref();

        node_tasks::dispatch_for_session(
            &conn, task_id, session_id, node_id, title, prompt, agent, model, now_ms,
        )
        .await

        }).await
    }
    async fn claim_next_node_task(
        &self,
        node_id: &str,
        now_ms: i64,
    ) -> Result<Option<NodeTaskRecord>> {
        let node_id = node_id.to_owned();
        self.db.write(move |conn| async move {
            let node_id = node_id.as_str();

        node_tasks::claim_next(&conn, node_id, now_ms).await

        }).await
    }
    async fn update_node_task_status(
        &self,
        task_id: &str,
        status: NodeTaskStatus,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<()> {
        let task_id = task_id.to_owned();
        let error = error.map(ToOwned::to_owned);
        self.db.write(move |conn| async move {
            let task_id = task_id.as_str();
            let error = error.as_deref();

        node_tasks::update_status(&conn, task_id, status, error, now_ms).await

        }).await
    }
    async fn request_node_task_cancel(&self, task_id: &str) -> Result<Option<NodeTaskStatus>> {
        let task_id = task_id.to_owned();
        self.db.write(move |conn| async move {
            let task_id = task_id.as_str();

        node_tasks::request_cancel(&conn, task_id).await

        }).await
    }
    async fn list_node_tasks(&self, node_id: &str, limit: u32) -> Result<Vec<NodeTaskRecord>> {
        let node_id = node_id.to_owned();
        self.db.read(move |conn| async move {
            let node_id = node_id.as_str();

        node_tasks::list_tasks(&conn, node_id, limit).await

        }).await
    }
    async fn get_node_task(&self, task_id: &str) -> Result<Option<NodeTaskRecord>> {
        let task_id = task_id.to_owned();
        self.db.read(move |conn| async move {
            let task_id = task_id.as_str();

        node_tasks::get_task(&conn, task_id).await

        }).await
    }
    async fn list_node_tasks_filtered(
        &self,
        node_id: Option<&str>,
        status: Option<NodeTaskStatus>,
        limit: u32,
    ) -> Result<Vec<NodeTaskRecord>> {
        let node_id = node_id.map(ToOwned::to_owned);
        self.db.read(move |conn| async move {
            let node_id = node_id.as_deref();

        node_tasks::list_tasks_filtered(&conn, node_id, status, limit).await

        }).await
    }
    async fn get_node_task_by_session(&self, session_id: &str) -> Result<Option<NodeTaskRecord>> {
        let session_id = session_id.to_owned();
        self.db.read(move |conn| async move {
            let session_id = session_id.as_str();

        node_tasks::get_by_session(&conn, session_id).await

        }).await
    }
    async fn converge_lost_node_tasks(
        &self,
        now_ms: i64,
        stale_ms: i64,
    ) -> Result<Vec<NodeTaskRecord>> {
        self.db.write(move |conn| async move {

        node_tasks::converge_lost(&conn, now_ms, stale_ms).await

        }).await
    }
    async fn clear_node_dialogs(&self, node_id: &str) -> Result<ClearNodeDialogs> {
        let node_id = node_id.to_owned();
        self.db.write(move |conn| async move {
            let node_id = node_id.as_str();

        node_tasks::clear_finished_sessions(&conn, node_id).await

        }).await
    }
    async fn delete_sessions(&self, ids: &[String]) -> Result<u64> {
        let ids = ids.to_owned();
        self.db.write(move |conn| async move {
            let ids = &ids;

        if ids.is_empty() {
            return Ok(0);
        }
        sessions::delete_many(&conn, ids).await

        }).await
    }
    async fn upsert_dag_def(&self, def: &DagDefRecord) -> Result<()> {
        let def = def.to_owned();
        self.db.write(move |conn| async move {
            let def = &def;

        dag::upsert_def(&conn, def).await

        }).await
    }
    async fn list_dag_defs(&self) -> Result<Vec<DagDefRecord>> {
        self.db.read(move |conn| async move {

        dag::list_defs(&conn).await

        }).await
    }
    async fn get_dag_def(&self, id: &str) -> Result<Option<DagDefRecord>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        dag::get_def(&conn, id).await

        }).await
    }
    async fn delete_dag_def(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();

        dag::delete_def(&conn, id).await

        }).await
    }
    async fn dispatch_dag_run(&self, run: &DagRunRecord) -> Result<DagRunRecord> {
        let run = run.to_owned();
        self.db.write(move |conn| async move {
            let run = &run;

        dag::dispatch(&conn, run).await

        }).await
    }
    async fn claim_next_dag_run(&self, node_id: &str, now_ms: i64) -> Result<Option<DagRunRecord>> {
        let node_id = node_id.to_owned();
        self.db.write(move |conn| async move {
            let node_id = node_id.as_str();

        dag::claim_next(&conn, node_id, now_ms).await

        }).await
    }
    async fn update_dag_run_status(
        &self,
        run_id: &str,
        status: opencoder_dag::DagRunStatus,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<DagRunRecord> {
        let run_id = run_id.to_owned();
        let error = error.map(ToOwned::to_owned);
        self.db.write(move |conn| async move {
            let run_id = run_id.as_str();
            let error = error.as_deref();

        dag::update_status(&conn, run_id, status, error, now_ms).await

        }).await
    }
        }
    };
}
