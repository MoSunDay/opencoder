//! 端到端集成：playbook 执行器的失败路径（与 `executor_playbook.rs` 共享
//! 场景基建）。覆盖：步骤失败坍缩（a 失败 ⇒ 下游不启动、无子 run 行、父
//! run Failed 但独立步骤照常完成）、todos 目标本地拒绝（平台语义）、未知
//! 剧本引用（load_spec 早期失败、无子行）。

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use opencoder_brain::playbook::{
    PlaybookOrigin, PlaybookSpec, PlaybookStep, PlaybookTarget, PlaybookTrigger,
};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_project::ProjectService;
use opencoder_store::{
    BrainPlaybookRecord, LibsqlStore, ProjectExecutorKind, ProjectStore, ProjectTodoRecord,
    ProjectTodoRunKind, ProjectTodoRunStatus, ProjectTodoStatus, Store,
};

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
    _keep: tempfile::TempDir,
}

async fn harness_on(
    store: Arc<LibsqlStore>,
    mock: Arc<MockChatClient>,
    brain: Option<opencoder_brain::Runtime>,
) -> Harness {
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
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
    Harness {
        service,
        store: store.clone(),
        projects: store,
        _keep: dir,
    }
}

async fn harness_with(scripts: Vec<Vec<LlmEvent>>) -> Harness {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut mock = MockChatClient::new();
    for script in scripts {
        mock = mock.push_script(script);
    }
    harness_on(store, Arc::new(mock), None).await
}

async fn seed_todo(h: &Harness, id: &str, executor_ref: &str) {
    let now = 1000;
    h.projects
        .create_todo(&ProjectTodoRecord {
            id: id.into(),
            milestone_id: None,
            title: format!("待办 {id}"),
            draft: "整理项目结构".into(),
            plan_md: Some("# 方案\n1. 落地目录约定".into()),
            status: ProjectTodoStatus::Planned,
            agent: "act".into(),
            executor_kind: ProjectExecutorKind::Playbook,
            executor_ref: Some(executor_ref.into()),
            executor_spec: None,
            active_session_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

/// 落一份剧本 spec 进 brain playbook 表（spec_json 即驱动读回的形态）。
async fn seed_playbook(h: &Harness, spec: &PlaybookSpec) {
    h.store
        .save_brain_playbook(&BrainPlaybookRecord {
            id: spec.id.clone(),
            name: spec.name.clone(),
            origin: "fixed".into(),
            situation_digest: None,
            spec_json: serde_json::to_string(spec).unwrap(),
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();
}

fn spec_of(id: &str, steps: Vec<PlaybookStep>) -> PlaybookSpec {
    PlaybookSpec {
        schema_version: opencoder_brain::playbook::spec::SCHEMA_VERSION,
        id: id.into(),
        name: format!("剧本 {id}"),
        origin: PlaybookOrigin::Fixed {},
        trigger: PlaybookTrigger::Manual {},
        steps,
    }
}

fn agent_step(name: &str, deps: &[&str], prompt: &str) -> PlaybookStep {
    PlaybookStep {
        name: name.into(),
        depends_on: deps.iter().map(|d| d.to_string()).collect(),
        target: PlaybookTarget::Agent {
            agent: "act".into(),
        },
        prompt: prompt.into(),
    }
}

fn dag_step(name: &str, deps: &[&str], dag: &str) -> PlaybookStep {
    PlaybookStep {
        name: name.into(),
        depends_on: deps.iter().map(|d| d.to_string()).collect(),
        target: PlaybookTarget::Dag { dag: dag.into() },
        prompt: "run".into(),
    }
}

async fn wait_run_done(
    projects: &Arc<dyn ProjectStore>,
    run_id: &str,
) -> opencoder_store::ProjectTodoRunRecord {
    let deadline = Instant::now() + Duration::from_secs(600);
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

/// 落库该 todo 的全部子 Step 行（按启动时间排序），附带步骤名
/// （input_snapshot.request.step）。
async fn step_rows(
    projects: &Arc<dyn ProjectStore>,
    todo_id: &str,
) -> Vec<(String, opencoder_store::ProjectTodoRunRecord)> {
    let mut rows: Vec<_> = projects
        .list_todo_runs(todo_id)
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.kind == ProjectTodoRunKind::Step)
        .map(|r| {
            let step = r
                .input_snapshot
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .and_then(|v| v["request"]["step"].as_str().map(str::to_string))
                .unwrap_or_default();
            (step, r)
        })
        .collect();
    rows.sort_by_key(|(_, r)| r.started_at);
    rows
}

#[tokio::test]
async fn playbook_step_failure_collapses_downstream() {
    // a 指向未登记 dag 定义（子 run 行 Failed）；b 依赖 a → 坍缩（无子行）；
    // c 独立 → Done；父 run Failed。
    let h = harness_with(vec![done("C")]).await;
    seed_playbook(
        &h,
        &spec_of(
            "pb-fail",
            vec![
                dag_step("a", &[], "missing-def"),
                agent_step("b", &["a"], "输出 B"),
                agent_step("c", &[], "输出 C"),
            ],
        ),
    )
    .await;
    seed_todo(&h, "t-fail", "pb-fail").await;

    let run_id = h.service.start_execute("t-fail").await.unwrap();
    let run = wait_run_done(&h.projects, &run_id).await;

    assert_eq!(run.status, ProjectTodoRunStatus::Failed);
    let out = run.output_md.unwrap();
    assert!(out.contains("- a: failed"), "a failure recorded: {out}");
    assert!(
        out.contains("dag definition not found"),
        "a error surfaced: {out}"
    );
    // 坍缩步骤没有子 run 行（从未启动）。
    let steps = step_rows(&h.projects, "t-fail").await;
    let names: Vec<&str> = steps.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["a", "c"], "b must never start");
    assert_eq!(steps[1].1.status, ProjectTodoRunStatus::Done);
    // todo 由父 run 收敛为 Failed。
    assert_eq!(
        h.projects.get_todo("t-fail").await.unwrap().unwrap().status,
        ProjectTodoStatus::Failed
    );
}

#[tokio::test]
async fn playbook_todos_target_is_rejected_locally() {
    // todos 工作流是平台语义：本地执行引擎在步骤解析期即失败，父 run
    // Failed 且消息指明需要控制面派发；无任何子 run 行。
    let h = harness_with(vec![]).await;
    seed_playbook(
        &h,
        &spec_of(
            "pb-todos",
            vec![PlaybookStep {
                name: "wf".into(),
                depends_on: vec![],
                target: PlaybookTarget::Todos {
                    workflow: "wf-main".into(),
                },
                prompt: "run".into(),
            }],
        ),
    )
    .await;
    seed_todo(&h, "t-todos", "pb-todos").await;

    let run_id = h.service.start_execute("t-todos").await.unwrap();
    let run = wait_run_done(&h.projects, &run_id).await;

    assert_eq!(run.status, ProjectTodoRunStatus::Failed);
    assert!(
        run.output_md
            .unwrap()
            .contains("todos target requires platform dispatch"),
        "todos rejection must name the platform dispatch requirement"
    );
    assert!(step_rows(&h.projects, "t-todos").await.is_empty());
}

#[tokio::test]
async fn playbook_missing_reference_fails_the_run() {
    // executor_ref 指向不存在的剧本 → 父 run 直接 Failed（无子行）。
    let h = harness_with(vec![]).await;
    seed_todo(&h, "t-missing", "pb-nope").await;

    let run_id = h.service.start_execute("t-missing").await.unwrap();
    let run = wait_run_done(&h.projects, &run_id).await;

    assert_eq!(run.status, ProjectTodoRunStatus::Failed);
    assert!(run
        .output_md
        .unwrap()
        .contains("playbook not found: pb-nope"));
    assert!(step_rows(&h.projects, "t-missing").await.is_empty());
}
