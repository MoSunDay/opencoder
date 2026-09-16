//! 端到端集成：todo 执行器维度（dag / team / brain，真 store +
//! MockChatClient）。覆盖：内联 dag spec 的本地工作流执行 + 工件落盘、
//! 内联 team spec 的本地讨论收敛、brain 钉住能力经路由表落到已登记 dag
//! 定义、节点缺 brain 运行时时在 claim 之前拒绝；控制面 override 经
//! 派发交接在无 brain 运行时的节点上直驱（D1）、override 禁止把 brain
//! 预解析成 brain、dag 解析早期失败不留悬挂 session 引用（D3①）。

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_project::ProjectService;
use opencoder_store::{
    LibsqlStore, ProjectExecutorKind, ProjectStore, ProjectTodoRecord, ProjectTodoRunStatus,
    ProjectTodoStatus, Store,
};
use serde_json::json;

fn done(text: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: text.into(),
        tool_calls: Vec::new(),
        usage: None,
    }]
}

struct Harness {
    service: Arc<ProjectService>,
    store: Arc<LibsqlStore>,
    projects: Arc<dyn ProjectStore>,
    dir: PathBuf,
    _keep: tempfile::TempDir,
}

async fn harness_on(
    store: Arc<LibsqlStore>,
    mock: Arc<MockChatClient>,
    brain: Option<opencoder_brain::Runtime>,
) -> Harness {
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    // Keep team data in this fixture as well: with no explicit team_root the
    // team executor falls back to `<global data root>/<workdir-hash>/team`,
    // which outlives the tempdir and litters the developer's global data
    // directory with per-test `project-t-team` leftovers. A project-level
    // `opencoder.json` (`Config::load` reads it from the workdir and merges
    // per key) pins `team_root` inside the fixture, so team data is removed
    // together with the tempdir just like the archived runs below.
    std::fs::write(
        dir.path().join("opencoder.json"),
        serde_json::to_vec(&json!({
            "team_root": dir.path().join("team").display().to_string(),
        }))
        .unwrap(),
    )
    .unwrap();
    let service = ProjectService::new();
    service
        .init(
            store.clone(),
            store.clone(),
            dir.path().to_path_buf(),
            Some(client),
            brain,
        )
        .await
        .unwrap();
    // Keep archived inputs and artifacts in this fixture, independent of the
    // developer's global data directory and unrelated disk writers.
    *service.require().unwrap().archive_root.lock().unwrap() = dir.path().join("runs");
    Harness {
        service,
        store: store.clone(),
        projects: store,
        dir: dir.path().to_path_buf(),
        _keep: dir,
    }
}

async fn harness_with(
    scripts: Vec<Vec<LlmEvent>>,
    brain: Option<opencoder_brain::Runtime>,
) -> Harness {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut mock = MockChatClient::new();
    for script in scripts {
        mock = mock.push_script(script);
    }
    harness_on(store, Arc::new(mock), brain).await
}

async fn seed_todo(
    projects: &Arc<dyn ProjectStore>,
    id: &str,
    kind: ProjectExecutorKind,
    executor_ref: Option<String>,
    executor_spec: Option<String>,
) {
    let now = 1000;
    projects
        .create_todo(&ProjectTodoRecord {
            id: id.into(),
            milestone_id: None,
            title: format!("待办 {id}"),
            draft: "整理项目结构".into(),
            plan_md: Some("# 方案\n1. 落地目录约定".into()),
            status: ProjectTodoStatus::Planned,
            agent: "act".into(),
            executor_kind: kind,
            executor_ref,
            executor_spec,
            active_session_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
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
            "run {run_id} did not finish; last output {:?}",
            run.output_md
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// 单步 agent 工作流 spec（type=agent 步）。
fn one_step_dag_spec() -> String {
    json!({"name":"项目单步","steps":[{"name":"run","kind":{"type":"agent","prompt":"输出 done"}}]})
        .to_string()
}

#[tokio::test]
async fn dag_executor_runs_inline_spec_and_writes_artifacts() {
    let h = harness_with(vec![done("done")], None).await;
    let todo_id = "t-dag".to_string();
    seed_todo(
        &h.projects,
        &todo_id,
        ProjectExecutorKind::Dag,
        None,
        Some(one_step_dag_spec()),
    )
    .await;

    let run_id = h.service.start_execute(&todo_id).await.unwrap();
    let run = wait_run_done(&h.projects, &run_id).await;

    assert_eq!(run.status, ProjectTodoRunStatus::Done);
    assert_eq!(run.executor_kind, ProjectExecutorKind::Dag);
    assert!(run.agent.starts_with("dag:"), "label {}", run.agent);
    let out_ref = run.output_ref.clone().expect("dag run output_ref");
    assert!(out_ref.contains("workflow"), "output_ref {out_ref}");
    let output = run.output_md.as_deref().unwrap_or("");
    assert!(output.contains("run"), "output_md {output:?}");
    // 宿主 session 以 run id 落库（DAG 事件挂在它名下）。
    assert!(h.store.get_session(&run_id).await.unwrap().is_some());
    // 工件：<workflow_root>/<run_id>/run/{output.txt, meta.json}
    let step_dir = PathBuf::from(&out_ref).join("run");
    assert!(step_dir.join("output.txt").exists(), "{step_dir:?}");
    assert!(step_dir.join("meta.json").exists(), "{step_dir:?}");
    let todo = h.projects.get_todo(&todo_id).await.unwrap().unwrap();
    assert_eq!(todo.status, ProjectTodoStatus::Done);
}

/// 存量 python 步骤的内联 spec 走专用迁移错误（fail-closed，绝不静默
/// 迁移），运行失败且错误信息含哨兵文案。
#[tokio::test]
async fn dag_executor_python_spec_fails_with_dedicated_migration_error() {
    let h = harness_with(vec![], None).await;
    let todo_id = "t-dag-python".to_string();
    seed_todo(
        &h.projects,
        &todo_id,
        ProjectExecutorKind::Dag,
        None,
        Some(
            json!({"name":"旧定义","steps":[{"name":"run","kind":{"type":"python","code":"pass"}}]})
                .to_string(),
        ),
    )
    .await;

    let run_id = h.service.start_execute(&todo_id).await.unwrap();
    let run = wait_run_done(&h.projects, &run_id).await;

    assert_eq!(run.status, ProjectTodoRunStatus::Failed);
    let output = run.output_md.as_deref().unwrap_or("");
    assert!(output.contains("已下线的 python 步骤"), "output {output:?}");
    // 会话未创建时不留悬挂引用。
    assert!(run.session_id.is_none(), "session_id {:?}", run.session_id);
}

#[tokio::test]
async fn team_executor_runs_inline_spec_to_completion() {
    // 调用顺序：队长 plan → 成员回答 → 队长 summary → 队长 closing。
    let plan =
        json!({"question":"目录怎么组织","participants":["act"],"rationale":"理由"}).to_string();
    let summary = json!({"summary":"全员一致","aligned":true,"ambiguities":[]}).to_string();
    let closing =
        json!({"complete":true,"next_question":null,"final_summary":"最终结论：按模块拆分"})
            .to_string();
    let h = harness_with(
        vec![
            done(&plan),
            done("成员意见：按模块分目录"),
            done(&summary),
            done(&closing),
        ],
        None,
    )
    .await;
    let spec = json!({
        "name": "服务端小队",
        "captain": {"node_id": "act", "name": "队长"},
        "members": [{"node_id": "act", "name": "成员", "capabilities": []}]
    })
    .to_string();
    let todo_id = "t-team".to_string();
    seed_todo(
        &h.projects,
        &todo_id,
        ProjectExecutorKind::Team,
        None,
        Some(spec),
    )
    .await;

    let run_id = h.service.start_execute(&todo_id).await.unwrap();
    let run = wait_run_done(&h.projects, &run_id).await;

    assert_eq!(run.status, ProjectTodoRunStatus::Done);
    assert_eq!(run.executor_kind, ProjectExecutorKind::Team);
    assert!(run.agent.starts_with("team:"), "label {}", run.agent);
    // output_ref 是 topic id（ULID），话题目录物化在 team_root 下。
    let topic_id = run.output_ref.clone().expect("team run output_ref");
    ulid::Ulid::from_string(&topic_id).expect("topic id is a ULID");
    let team_root = h.dir.join("team");
    let topic_dir = team_root.join("project-t-team").join(&topic_id);
    assert!(topic_dir.is_dir(), "{topic_dir:?}");
    assert_eq!(
        run.output_md.as_deref(),
        Some("最终结论：按模块拆分"),
        "final summary is the run output"
    );
    let todo = h.projects.get_todo(&todo_id).await.unwrap().unwrap();
    assert_eq!(todo.status, ProjectTodoStatus::Done);
}

#[tokio::test]
async fn legacy_brain_todos_reject_before_claim_even_with_a_pinned_target_or_override() {
    let h = harness_with(vec![], None).await;
    for (id, reference) in [
        ("legacy-empty", None),
        ("legacy-pinned", Some("cap".into())),
    ] {
        seed_todo(&h.projects, id, ProjectExecutorKind::Brain, reference, None).await;
        assert!(h
            .service
            .start_execute(id)
            .await
            .unwrap_err()
            .to_string()
            .contains("migration required"));
        let ov = serde_json::from_value(json!({"kind":"agent","ref":"act"})).unwrap();
        assert!(h.service.start_execute_with(id, Some(ov)).await.is_err());
        assert!(h.projects.list_todo_runs(id).await.unwrap().is_empty());
        assert_eq!(
            h.projects.get_todo(id).await.unwrap().unwrap().status,
            ProjectTodoStatus::Planned
        );
    }
}

/// D3①：dag 解析在宿主 session 创建之前失败（引用了不存在的 dag 定义）
/// → run Failed 且 session_id 为空，store 里没有以 run id 为 id 的会话
/// （不悬挂引用）。
#[tokio::test]
async fn dag_early_failure_leaves_no_dangling_session_id() {
    let h = harness_with(vec![], None).await;
    let todo_id = "t-dag-missing".to_string();
    seed_todo(
        &h.projects,
        &todo_id,
        ProjectExecutorKind::Dag,
        Some("no-such-dag".to_string()),
        None,
    )
    .await;

    let run_id = h.service.start_execute(&todo_id).await.unwrap();
    let run = wait_run_done(&h.projects, &run_id).await;

    assert_eq!(run.status, ProjectTodoRunStatus::Failed);
    assert!(
        run.output_md
            .as_deref()
            .unwrap_or("")
            .contains("no-such-dag"),
        "output {:?}",
        run.output_md
    );
    assert!(run.session_id.is_none(), "宿主会话未创建：不留悬挂引用");
    assert!(h.store.get_session(&run_id).await.unwrap().is_none());
    let todo = h.projects.get_todo(&todo_id).await.unwrap().unwrap();
    assert_eq!(todo.status, ProjectTodoStatus::Failed);
}
