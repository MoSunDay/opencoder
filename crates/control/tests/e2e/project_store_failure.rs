//! Project store failure paths: an injected `get_todo` fault must surface
//! as a 500 from `execute`'s executor admission, before any fleet call.

use std::sync::Arc;

use opencoder_store::{
    PayloadChunkRecord, ProjectGoalPatch, ProjectGoalRecord, ProjectInitiativePatch,
    ProjectInitiativeRecord, ProjectStore, ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunPage,
    ProjectTodoRunPatch, ProjectTodoRunRecord, ProjectTodoRunStatus, ProjectTodoStatus,
};
use reqwest::Method;

use crate::support::Harness;

/// Fault-injection wrapper: delegates every call to `inner` except `get_todo`,
/// which fails with a fixed error (same style as the project crate's
/// `CreateRunFailingStore`).
struct GetTodoFailingStore {
    inner: Arc<dyn ProjectStore>,
}

#[async_trait::async_trait]
impl ProjectStore for GetTodoFailingStore {
    fn project_backend_name(&self) -> &'static str {
        self.inner.project_backend_name()
    }
    async fn list_todo_assignments(
        &self,
        todo_id: &str,
    ) -> anyhow::Result<Vec<opencoder_store::project::ProjectAssignment>> {
        self.inner.list_todo_assignments(todo_id).await
    }
    async fn latest_todo_assignments(
        &self,
    ) -> anyhow::Result<Vec<opencoder_store::project::ProjectAssignment>> {
        self.inner.latest_todo_assignments().await
    }
    async fn link_todo_execution(
        &self,
        assignment: &opencoder_store::project::ProjectAssignment,
    ) -> anyhow::Result<()> {
        self.inner.link_todo_execution(assignment).await
    }
    async fn unlink_todo_execution(
        &self,
        todo_id: &str,
        execution_id: &str,
    ) -> anyhow::Result<bool> {
        self.inner
            .unlink_todo_execution(todo_id, execution_id)
            .await
    }
    async fn create_goal(&self, rec: &ProjectGoalRecord) -> anyhow::Result<()> {
        self.inner.create_goal(rec).await
    }
    async fn patch_goal(
        &self,
        id: &str,
        patch: &ProjectGoalPatch,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        self.inner.patch_goal(id, patch, now_ms).await
    }
    async fn delete_goal(&self, id: &str) -> anyhow::Result<bool> {
        self.inner.delete_goal(id).await
    }
    async fn list_goals(&self) -> anyhow::Result<Vec<ProjectGoalRecord>> {
        self.inner.list_goals().await
    }
    async fn create_initiative(&self, rec: &ProjectInitiativeRecord) -> anyhow::Result<()> {
        self.inner.create_initiative(rec).await
    }
    async fn patch_initiative(
        &self,
        id: &str,
        patch: &ProjectInitiativePatch,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        self.inner.patch_initiative(id, patch, now_ms).await
    }
    async fn delete_initiative(&self, id: &str) -> anyhow::Result<bool> {
        self.inner.delete_initiative(id).await
    }
    async fn list_initiatives(
        &self,
        goal_id: Option<&str>,
    ) -> anyhow::Result<Vec<ProjectInitiativeRecord>> {
        self.inner.list_initiatives(goal_id).await
    }
    async fn create_todo(&self, rec: &ProjectTodoRecord) -> anyhow::Result<()> {
        self.inner.create_todo(rec).await
    }
    async fn patch_todo(
        &self,
        id: &str,
        patch: &ProjectTodoPatch,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        self.inner.patch_todo(id, patch, now_ms).await
    }
    async fn claim_todo_running(&self, id: &str, now_ms: i64) -> anyhow::Result<bool> {
        self.inner.claim_todo_running(id, now_ms).await
    }
    async fn claim_todo_running_with_run(
        &self,
        rec: &ProjectTodoRunRecord,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        self.inner.claim_todo_running_with_run(rec, now_ms).await
    }
    async fn patch_todo_when(
        &self,
        id: &str,
        when: ProjectTodoStatus,
        patch: &ProjectTodoPatch,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        self.inner.patch_todo_when(id, when, patch, now_ms).await
    }
    async fn delete_todo(&self, id: &str) -> anyhow::Result<bool> {
        self.inner.delete_todo(id).await
    }
    async fn get_todo(&self, _id: &str) -> anyhow::Result<Option<ProjectTodoRecord>> {
        Err(anyhow::anyhow!("injected get_todo failure"))
    }
    async fn list_todos(
        &self,
        initiative_id: Option<&str>,
    ) -> anyhow::Result<Vec<ProjectTodoRecord>> {
        self.inner.list_todos(initiative_id).await
    }
    async fn create_todo_run(&self, rec: &ProjectTodoRunRecord) -> anyhow::Result<()> {
        self.inner.create_todo_run(rec).await
    }
    async fn patch_todo_run(
        &self,
        id: &str,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        self.inner.patch_todo_run(id, patch, now_ms).await
    }
    async fn patch_todo_run_when(
        &self,
        id: &str,
        when: ProjectTodoRunStatus,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        self.inner
            .patch_todo_run_when(id, when, patch, now_ms)
            .await
    }
    async fn get_todo_run(&self, id: &str) -> anyhow::Result<Option<ProjectTodoRunRecord>> {
        self.inner.get_todo_run(id).await
    }
    async fn list_todo_runs(&self, todo_id: &str) -> anyhow::Result<Vec<ProjectTodoRunRecord>> {
        self.inner.list_todo_runs(todo_id).await
    }
    async fn list_todo_runs_page(
        &self,
        todo_id: &str,
        before_version: Option<i64>,
        limit: u32,
    ) -> anyhow::Result<ProjectTodoRunPage> {
        self.inner
            .list_todo_runs_page(todo_id, before_version, limit)
            .await
    }
    async fn project_text_chunk(
        &self,
        record_kind: &str,
        owner_id: &str,
        id: &str,
        field: &str,
        offset: u64,
        max_bytes: usize,
    ) -> anyhow::Result<Option<PayloadChunkRecord>> {
        self.inner
            .project_text_chunk(record_kind, owner_id, id, field, offset, max_bytes)
            .await
    }
    async fn list_running_todo_runs(&self) -> anyhow::Result<Vec<ProjectTodoRunRecord>> {
        self.inner.list_running_todo_runs().await
    }
    async fn next_todo_version(&self, todo_id: &str) -> anyhow::Result<i64> {
        self.inner.next_todo_version(todo_id).await
    }
}

#[tokio::test]
async fn execute_reports_500_when_todo_store_fails() {
    let dir = tempfile::tempdir().unwrap();
    let inner: Arc<dyn ProjectStore> = Arc::new(
        opencoder_store::LibsqlStore::open(dir.path().join("proj.db"))
            .await
            .unwrap(),
    );
    let h = Harness::with_projects(Arc::new(GetTodoFailingStore { inner })).await;

    // `execute` checks the stored executor via `projects.get_todo` before any
    // fleet/index call; the injected store fault must become a 500 carrying
    // both the branch tag and the underlying error text.
    let (status, body) = h
        .req(Method::POST, "/api/project/todos/t-gone/execute", None)
        .await;
    assert_eq!(status, 500, "{body}");
    let error = body["error"].as_str().unwrap_or_default();
    assert!(error.contains("load todo"), "{body}");
    assert!(error.contains("injected get_todo failure"), "{body}");
}
