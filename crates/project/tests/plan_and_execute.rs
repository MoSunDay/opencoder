//! 端到端集成：plan/execute 直驱运行（真 store + MockChatClient）。
//! 覆盖方案生成回写、执行输出持久化 + 同会话续跑（持续推进）、中途取消
//! 回退 Planned、前置拒绝与 overview 树形结构。

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient};
use opencoder_project::ProjectService;
use opencoder_store::{
    LibsqlStore, ProjectGoalRecord, ProjectGoalStatus, ProjectMilestoneRecord,
    ProjectMilestoneStatus, ProjectStore, ProjectTodoRecord, ProjectTodoPatch, ProjectTodoStatus,
    ProjectTodoRunKind, ProjectTodoRunStatus, Store, TASK_TYPE_PROJECT,
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
    let service = ProjectService::new();
    service
        .init(
            store.clone(),
            store.clone(),
            dir.path().to_path_buf(),
            Some(client),
        )
        .await
        .unwrap();
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

async fn seed_milestone(projects: &Arc<dyn ProjectStore>, id: &str, goal_id: &str) {
    let now = 1000;
    projects
        .create_milestone(&ProjectMilestoneRecord {
            id: id.into(),
            goal_id: goal_id.into(),
            title: format!("里程碑 {id}"),
            detail_md: None,
            status: ProjectMilestoneStatus::Planned,
            sort: 0,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

async fn seed_todo(projects: &Arc<dyn ProjectStore>, id: &str, milestone_id: Option<&str>) -> String {
    let now = 1000;
    projects
        .create_todo(&ProjectTodoRecord {
            id: id.into(),
            milestone_id: milestone_id.map(Into::into),
            title: format!("待办 {id}"),
            draft: "做一个计数器".into(),
            plan_md: None,
            status: ProjectTodoStatus::Draft,
            agent: "act".into(),
            active_session_id: None,
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

#[tokio::test]
async fn plan_generates_and_updates_todo() {
    let h = harness(vec![done("# 实施计划\n1. 步骤一")]).await;
    seed_goal(&h.projects, "g1").await;
    seed_milestone(&h.projects, "m1", "g1").await;
    let todo_id = seed_todo(&h.projects, "t1", Some("m1")).await;

    let run_id = h.service.start_plan(&todo_id).await.unwrap();
    let run = wait_run_done(&h.projects, &run_id).await;

    assert_eq!(run.status, ProjectTodoRunStatus::Done);
    assert_eq!(run.kind, ProjectTodoRunKind::Plan);
    assert_eq!(run.version, 1);
    assert_eq!(run.agent, "plan");
    assert_eq!(run.output_md.as_deref(), Some("# 实施计划\n1. 步骤一"));
    let session_id = run.session_id.expect("plan run session");
    let meta = h.store.get_session(&session_id).await.unwrap().unwrap();
    assert_eq!(meta.task_type.as_deref(), Some(TASK_TYPE_PROJECT));
    assert_eq!(meta.agent.as_deref(), Some("plan"));

    let todo = h.projects.get_todo(&todo_id).await.unwrap().unwrap();
    assert_eq!(todo.plan_md.as_deref(), Some("# 实施计划\n1. 步骤一"));
    assert_eq!(todo.status, ProjectTodoStatus::Planned);
}

#[tokio::test]
async fn execute_runs_and_persists_output_then_rerun_resumes_same_session() {
    let h = harness(vec![
        done("# 方案\n1. 写代码"),
        tool_turn("先跑一步", "echo ok"),
        done("全部完成"),
    ])
    .await;
    let todo_id = seed_todo(&h.projects, "t1", None).await;

    // v1：生成方案。
    let plan_run = h.service.start_plan(&todo_id).await.unwrap();
    let plan_run = wait_run_done(&h.projects, &plan_run).await;
    assert_eq!(plan_run.version, 1);
    assert_eq!(plan_run.status, ProjectTodoRunStatus::Done);

    // v2：首次执行（含一次真实 bash 工具回合）。
    let exec1 = h.service.start_execute(&todo_id).await.unwrap();
    let exec1 = wait_run_done(&h.projects, &exec1).await;
    assert_eq!(exec1.kind, ProjectTodoRunKind::Execute);
    assert_eq!(exec1.version, 2);
    assert_eq!(exec1.status, ProjectTodoRunStatus::Done);
    assert_eq!(exec1.output_md.as_deref(), Some("全部完成"));
    let sid1 = exec1.session_id.clone().expect("execute session");
    let todo = h.projects.get_todo(&todo_id).await.unwrap().unwrap();
    assert_eq!(todo.status, ProjectTodoStatus::Done);
    assert_eq!(todo.active_session_id.as_deref(), Some(sid1.as_str()));
    let after_first = h.store.load_messages(&sid1).await.unwrap().len();
    assert!(after_first >= 3, "expected >=3 messages, got {after_first}");

    // v3：续跑必须 resume 同一 session，消息只增不换。
    h.mock.queue_script(tool_turn("继续", "echo again"));
    h.mock.queue_script(done("再次完成"));
    let exec2 = h.service.start_execute(&todo_id).await.unwrap();
    let exec2 = wait_run_done(&h.projects, &exec2).await;
    assert_eq!(exec2.version, 3);
    assert_eq!(exec2.status, ProjectTodoRunStatus::Done);
    assert_eq!(exec2.output_md.as_deref(), Some("再次完成"));
    let todo = h.projects.get_todo(&todo_id).await.unwrap().unwrap();
    assert_eq!(todo.status, ProjectTodoStatus::Done);
    assert_eq!(
        todo.active_session_id.as_deref(),
        Some(sid1.as_str()),
        "rerun must resume the same session"
    );
    let after_second = h.store.load_messages(&sid1).await.unwrap().len();
    assert!(
        after_second > after_first,
        "resumed session must keep growing: {after_first} -> {after_second}"
    );
}

#[tokio::test]
async fn cancel_midflight_reverts_todo_to_planned() {
    // 取消路径依赖 session runner 的 select! 取消臂：硬取消会中断在途
    // LLM 流并让 run() 以 Ok 收场（空回合不落 assistant 消息），因此
    // 「Ok 但无新输出 + cancel 已触发」必须判为 Cancelled，todo 回 Planned。
    let hang = Arc::new(tokio::sync::Notify::new());
    let h = harness(vec![]).await;
    h.mock.queue_hang(hang.clone());
    let todo_id = seed_todo(&h.projects, "t1", None).await;
    h.projects
        .patch_todo(
            &todo_id,
            &ProjectTodoPatch {
                plan_md: Some(Some("# 方案".into())),
                status: Some(ProjectTodoStatus::Planned),
                ..Default::default()
            },
            2000,
        )
        .await
        .unwrap();

    let run_id = h.service.start_execute(&todo_id).await.unwrap();
    let calls = h.mock.clone();
    wait_until("in-flight LLM call", move || calls.call_count() >= 1).await;
    assert!(h.service.cancel(&run_id).await.unwrap());

    let run = wait_run_done(&h.projects, &run_id).await;
    assert_eq!(run.status, ProjectTodoRunStatus::Cancelled);
    let todo = h.projects.get_todo(&todo_id).await.unwrap().unwrap();
    assert_eq!(todo.status, ProjectTodoStatus::Planned);
    assert!(!h.service.cancel(&run_id).await.unwrap());
}

#[tokio::test]
async fn start_execute_rejects_unplanned_and_running_todos() {
    let h = harness(vec![]).await;
    let unplanned = seed_todo(&h.projects, "t-no-plan", None).await;
    let err = h.service.start_execute(&unplanned).await.unwrap_err();
    assert!(err.to_string().contains("no plan"), "got: {err:#}");

    let running = seed_todo(&h.projects, "t-running", None).await;
    h.projects
        .patch_todo(
            &running,
            &ProjectTodoPatch {
                plan_md: Some(Some("# 方案".into())),
                status: Some(ProjectTodoStatus::Running),
                ..Default::default()
            },
            2000,
        )
        .await
        .unwrap();
    let err = h.service.start_execute(&running).await.unwrap_err();
    assert!(err.to_string().contains("running"), "got: {err:#}");
    let err = h.service.start_plan(&running).await.unwrap_err();
    assert!(err.to_string().contains("running"), "got: {err:#}");

    let err = h.service.start_execute("missing").await.unwrap_err();
    assert!(err.to_string().contains("todo not found"), "got: {err:#}");
}

#[tokio::test]
async fn overview_tree_shape() {
    let h = harness(vec![]).await;
    seed_goal(&h.projects, "g1").await;
    seed_milestone(&h.projects, "m1", "g1").await;
    seed_todo(&h.projects, "t-in", Some("m1")).await;
    seed_todo(&h.projects, "t-backlog", None).await;

    let tree = h.service.overview().await.unwrap();
    assert_eq!(tree["goals"].as_array().unwrap().len(), 1);
    let goal = &tree["goals"][0];
    assert_eq!(goal["id"], "g1");
    let milestones = goal["milestones"].as_array().unwrap();
    assert_eq!(milestones.len(), 1);
    assert_eq!(milestones[0]["id"], "m1");
    let todos = milestones[0]["todos"].as_array().unwrap();
    assert_eq!(todos.len(), 1);
    assert_eq!(todos[0]["id"], "t-in");
    let backlog = tree["backlog"].as_array().unwrap();
    assert_eq!(backlog.len(), 1);
    assert_eq!(backlog[0]["id"], "t-backlog");
}
