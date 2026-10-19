macro_rules! project_implementation_1 {
    ($($methods:tt)*) => {
        project_implementation_finish! {
            $($methods)*
    async fn patch_todo(&self, id: &str, patch: &ProjectTodoPatch, now_ms: i64) -> Result<bool> {
        let id = id.to_owned();
        let patch = patch.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();
            let patch = &patch;

        tags::patch_todo(&conn, id, patch, None, None, now_ms).await

        }).await
    }
    async fn claim_todo_running(&self, id: &str, now_ms: i64) -> Result<bool> {
        let id = id.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();

        super::project_runs::claim_todo_running(&conn, id, now_ms).await

        }).await
    }
    async fn claim_todo_running_with_run(
        &self,
        rec: &ProjectTodoRunRecord,
        now_ms: i64,
    ) -> Result<bool> {
        let rec = rec.to_owned();
        self.db.write(move |conn| async move {
            let rec = &rec;

        super::project_runs::claim_todo_running_with_run(&conn, rec, now_ms).await

        }).await
    }
    async fn patch_todo_when(
        &self,
        id: &str,
        when: ProjectTodoStatus,
        patch: &ProjectTodoPatch,
        now_ms: i64,
    ) -> Result<bool> {
        let id = id.to_owned();
        let patch = patch.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();
            let patch = &patch;

        super::tx::run_tx(&conn, "BEGIN IMMEDIATE", || {
            super::project_runs::patch_todo_when(&conn, id, when, patch, now_ms)
        })
        .await

        }).await
    }
    async fn delete_todo(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();

        super::project_runs::delete_todo(&conn, id).await

        }).await
    }
    async fn get_todo(&self, id: &str) -> Result<Option<ProjectTodoRecord>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        super::project_runs::get_todo(&conn, id).await

        }).await
    }
    async fn reorder_todos(
        &self,
        initiative_id: Option<&str>,
        board_status: &str,
        ids: &[String],
        now_ms: i64,
    ) -> Result<()> {
        let initiative_id = initiative_id.map(ToOwned::to_owned);
        let board_status = board_status.to_owned();
        let ids = ids.to_owned();
        self.db.write(move |conn| async move {
            let initiative_id = initiative_id.as_deref();
            let board_status = board_status.as_str();
            let ids = &ids;

        super::project_runs::reorder_todos(&conn, initiative_id, board_status, ids, now_ms).await

        }).await
    }
    async fn get_todo_summary(&self, id: &str) -> Result<Option<crate::ProjectTodoSummary>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        super::project_runs::get_todo_summary(&conn, id).await

        }).await
    }
    async fn list_todos(&self, initiative_id: Option<&str>) -> Result<Vec<ProjectTodoRecord>> {
        let initiative_id = initiative_id.map(ToOwned::to_owned);
        self.db.read(move |conn| async move {
            let initiative_id = initiative_id.as_deref();

        super::project_runs::list_todos(&conn, initiative_id).await

        }).await
    }
    async fn create_todo_run(&self, rec: &ProjectTodoRunRecord) -> Result<()> {
        let rec = rec.to_owned();
        self.db.write(move |conn| async move {
            let rec = &rec;

        super::project_runs::create_todo_run(&conn, rec).await

        }).await
    }
    async fn finish_todo_run(
        &self,
        id: &str,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool> {
        let id = id.to_owned();
        let patch = patch.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();
            let patch = &patch;

        super::project_runs::finish_todo_run(&conn, id, patch, now_ms).await

        }).await
    }
    async fn patch_todo_run(
        &self,
        id: &str,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool> {
        let id = id.to_owned();
        let patch = patch.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();
            let patch = &patch;

        super::project_runs::patch_todo_run(&conn, id, patch, now_ms).await

        }).await
    }
    async fn patch_todo_run_when(
        &self,
        id: &str,
        when: ProjectTodoRunStatus,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool> {
        let id = id.to_owned();
        let patch = patch.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();
            let patch = &patch;

        super::project_runs::patch_todo_run_when(&conn, id, when, patch, now_ms).await

        }).await
    }
    async fn get_todo_run(&self, id: &str) -> Result<Option<ProjectTodoRunRecord>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        super::project_runs::get_todo_run(&conn, id).await

        }).await
    }
    async fn get_todo_run_summary(&self, id: &str) -> Result<Option<crate::ProjectTodoRunSummary>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        super::project_runs::get_todo_run_summary(&conn, id).await

        }).await
    }
    async fn list_todo_runs(&self, todo_id: &str) -> Result<Vec<ProjectTodoRunRecord>> {
        let todo_id = todo_id.to_owned();
        self.db.read(move |conn| async move {
            let todo_id = todo_id.as_str();

        super::project_runs::list_todo_runs(&conn, todo_id).await

        }).await
    }
    async fn list_todo_runs_page(
        &self,
        todo_id: &str,
        before_version: Option<i64>,
        limit: u32,
    ) -> Result<crate::ProjectTodoRunPage> {
        let todo_id = todo_id.to_owned();
        self.db.read(move |conn| async move {
            let todo_id = todo_id.as_str();

        super::project_runs::list_todo_runs_page(&conn, todo_id, before_version, limit).await

        }).await
    }
    async fn project_text_chunk(
        &self,
        record_kind: &str,
        owner_id: &str,
        id: &str,
        field: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<crate::PayloadChunkRecord>> {
        let record_kind = record_kind.to_owned();
        let owner_id = owner_id.to_owned();
        let id = id.to_owned();
        let field = field.to_owned();
        self.db.read(move |conn| async move {
            let record_kind = record_kind.as_str();
            let owner_id = owner_id.as_str();
            let id = id.as_str();
            let field = field.as_str();

        super::project_runs::project_text_chunk(
            &conn,
            record_kind,
            owner_id,
            id,
            field,
            offset,
            max_bytes,
        )
        .await

        }).await
    }
    async fn list_running_todo_runs(&self) -> Result<Vec<ProjectTodoRunRecord>> {
        self.db.read(move |conn| async move {

        super::project_runs::list_running_todo_runs(&conn).await

        }).await
    }
    async fn next_todo_version(&self, todo_id: &str) -> Result<i64> {
        let todo_id = todo_id.to_owned();
        self.db.read(move |conn| async move {
            let todo_id = todo_id.as_str();

        super::project_runs::next_todo_version(&conn, todo_id).await

        }).await
    }
        }
    };
}
