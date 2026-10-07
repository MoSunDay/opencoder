//! 端到端集成：plan/execute 直驱运行（真 store + MockChatClient）。
//! 覆盖方案生成回写、执行输出持久化 + 同会话续跑（持续推进）、中途取消
//! 回退 Planned、前置拒绝与 overview 树形结构、execute run 启动时落
//! plan_md 方案快照。

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient};
use opencoder_project::ProjectService;
use opencoder_store::{
    LibsqlStore, ProjectExecutorKind, ProjectGoalPatch, ProjectGoalRecord, ProjectGoalStatus,
    ProjectInitiativePatch, ProjectInitiativeRecord, ProjectInitiativeStatus, ProjectStore,
    ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunPatch,
    ProjectTodoRunRecord, ProjectTodoRunStatus, ProjectTodoStatus, Store, TASK_TYPE_PROJECT,
};

fn done(text: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: text.into(),
        tool_calls: Vec::new(),
        usage: None,
    }]
}

fn tool_turn(text: &str, command: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![CompletedToolCall {
            id: "t1".into(),
            name: "bash".into(),
            input: serde_json::json!({ "command": command }),
        }],
        usage: None,
    }]
}

struct Harness {
    service: Arc<ProjectService>,
    mock: Arc<MockChatClient>,
    store: Arc<dyn Store>,
    projects: Arc<dyn ProjectStore>,
    _dir: tempfile::TempDir,
}

async fn harness(scripts: Vec<Vec<LlmEvent>>) -> Harness {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut mock = MockChatClient::new();
    for script in scripts {
        mock = mock.push_script(script);
    }
    let mock = Arc::new(mock);
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("opencoder.json"),
        r#"{"local_memory":false}"#,
    )
    .unwrap();
    let service = ProjectService::new();
    service
        .init(
            store.clone(),
            store.clone(),
            dir.path().to_path_buf(),
            Some(client),
            None,
        )
        .await
        .unwrap();
    // Keep archived inputs and artifacts in this fixture, independent of the
    // developer's global data directory and unrelated disk writers.
    *service.require().unwrap().archive_root.lock().unwrap() = dir.path().join("runs");
    Harness {
        service,
        mock,
        store: store.clone(),
        projects: store,
        _dir: dir,
    }
}

async fn seed_goal(projects: &Arc<dyn ProjectStore>, id: &str) {
    let now = 1000;
    projects
        .create_goal(&ProjectGoalRecord {
            id: id.into(),
            title: format!("目标 {id}"),
            detail_md: Some("目标说明".into()),
            status: ProjectGoalStatus::Active,
            sort: 0,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

async fn seed_initiative(projects: &Arc<dyn ProjectStore>, id: &str, goal_id: &str) {
    let now = 1000;
    projects
        .create_initiative(&ProjectInitiativeRecord {
            id: id.into(),
            goal_id: Some(goal_id.into()),
            title: format!("专项 {id}"),
            detail_md: None,
            status: ProjectInitiativeStatus::Planned,
            sort: 0,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

async fn seed_todo(
    projects: &Arc<dyn ProjectStore>,
    id: &str,
    initiative_id: Option<&str>,
) -> String {
    let now = 1000;
    projects
        .create_todo(&ProjectTodoRecord {
            id: id.into(),
            initiative_id: initiative_id.map(Into::into),
            title: format!("待办 {id}"),
            draft: "做一个计数器".into(),
            plan_md: None,
            status: ProjectTodoStatus::Draft,
            agent: "act".into(),
            executor_kind: ProjectExecutorKind::Agent,
            executor_ref: None,
            executor_spec: None,
            active_session_id: None,
            board_status: "backlog".into(),
            position: 0,
            capability_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    id.to_string()
}

async fn wait_run_done(
    projects: &Arc<dyn ProjectStore>,
    run_id: &str,
) -> opencoder_store::ProjectTodoRunRecord {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let run = projects
            .get_todo_run(run_id)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("run row missing: {run_id}"));
        if run.status != ProjectTodoRunStatus::Running {
            return run;
        }
        assert!(
            Instant::now() < deadline,
            "run {run_id} did not finish; last status {:?}",
            run.status
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_until(what: &str, mut probe: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !probe() {
        assert!(Instant::now() < deadline, "condition not met: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// 故障注入包装：原子 claim + run 入口在任何写入前失败。Store 层另有
/// 真实唯一键冲突用例验证事务内 INSERT 失败会回滚已经执行的 claim。
struct CreateRunFailingStore {
    inner: Arc<LibsqlStore>,
}

#[async_trait::async_trait]
impl ProjectStore for CreateRunFailingStore {
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
        _rec: &ProjectTodoRunRecord,
        _now_ms: i64,
    ) -> anyhow::Result<bool> {
        Err(anyhow::anyhow!("injected atomic claim failure"))
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
    async fn get_todo(&self, id: &str) -> anyhow::Result<Option<ProjectTodoRecord>> {
        self.inner.get_todo(id).await
    }
    async fn list_todos(
        &self,
        initiative_id: Option<&str>,
    ) -> anyhow::Result<Vec<ProjectTodoRecord>> {
        self.inner.list_todos(initiative_id).await
    }
    async fn create_todo_run(&self, _rec: &ProjectTodoRunRecord) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("injected create failure"))
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
    async fn list_running_todo_runs(&self) -> anyhow::Result<Vec<ProjectTodoRunRecord>> {
        self.inner.list_running_todo_runs().await
    }
    async fn next_todo_version(&self, todo_id: &str) -> anyhow::Result<i64> {
        self.inner.next_todo_version(todo_id).await
    }
}

#[path = "replay/contracts.rs"]
mod replay;

#[path = "plan_and_execute/suite_1.rs"]
mod suite_1;
