//! 端到端集成：playbook 执行器的成功路径（本地剧本编排轨，brain 双轨
//! 调度落地端，真 store + MockChatClient）。覆盖：串行链按拓扑序执行并逐段
//! 落子 run 行、菱形依赖并发调度（b/c 同批、d 收口）、brain 钉住能力步骤
//! 经派发交接落到默认路由（无 brain 运行时亦可跑）。失败路径见
//! `executor_playbook_errors.rs`。

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
async fn playbook_runs_serial_chain_in_topo_order() {
    // a → b → c：三段 agent 步骤按序执行；每段落一条 Step 子 run 行，
    // 父 run（Execute）最后关闭且独占 todo 状态回写。
    let h = harness_with(vec![done("A"), done("B"), done("C")]).await;
    seed_playbook(
        &h,
        &spec_of(
            "pb-chain",
            vec![
                agent_step("a", &[], "第一步：输出 A"),
                agent_step("b", &["a"], "第二步：输出 B"),
                agent_step("c", &["b"], "第三步：输出 C"),
            ],
        ),
    )
    .await;
    seed_todo(&h, "t-chain", "pb-chain").await;

    let run_id = h.service.start_execute("t-chain").await.unwrap();
    let run = wait_run_done(&h.projects, &run_id).await;

    assert_eq!(run.status, ProjectTodoRunStatus::Done);
    assert_eq!(run.kind, ProjectTodoRunKind::Execute);
    assert_eq!(run.executor_kind, ProjectExecutorKind::Playbook);
    let todo = h.projects.get_todo("t-chain").await.unwrap().unwrap();

    assert_eq!(todo.status, ProjectTodoStatus::Done);

    // 子 Step 行：每步一条，started_at 严格递增（串行链）。
    let steps = step_rows(&h.projects, "t-chain").await;
    let names: Vec<&str> = steps.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["a", "b", "c"]);
    for pair in steps.windows(2) {
        assert!(
            pair[0].1.started_at <= pair[1].1.started_at,
            "serial chain must start in topo order"
        );
    }
    assert!(steps[0].1.finished_at.is_some());
    // 渲染后的步骤指令进入子行 input_snapshot.prompt 留痕（子行的
    // plan_md 列不承载步骤指令——那是 Execute 行的字段）。
    let snap: serde_json::Value =
        serde_json::from_str(steps[0].1.input_snapshot.as_deref().unwrap()).unwrap();
    assert!(snap["prompt"].as_str().unwrap().contains("第一步"));
    assert!(snap["request"]["step"].as_str().unwrap() == "a");

    // 父 run 汇总按拓扑序逐步骤一行。
    let out = run.output_md.unwrap();
    let ia = out.find("- a: done").expect("a line");
    let ib = out.find("- b: done").expect("b line");
    let ic = out.find("- c: done").expect("c line");
    assert!(ia < ib && ib < ic, "summary topo order: {out}");
}

#[tokio::test]
async fn playbook_runs_diamond_with_parallel_waves() {
    // a → b,c → d：b/c 同批并发（都在 a 之后、d 之前启动），d 最后。
    let h = harness_with(vec![done("A"), done("B"), done("C"), done("D")]).await;
    seed_playbook(
        &h,
        &spec_of(
            "pb-diamond",
            vec![
                agent_step("a", &[], "输出 A"),
                agent_step("b", &["a"], "输出 B"),
                agent_step("c", &["a"], "输出 C"),
                agent_step("d", &["b", "c"], "输出 D"),
            ],
        ),
    )
    .await;
    seed_todo(&h, "t-diamond", "pb-diamond").await;

    let run_id = h.service.start_execute("t-diamond").await.unwrap();
    let run = wait_run_done(&h.projects, &run_id).await;
    assert_eq!(run.status, ProjectTodoRunStatus::Done);

    let steps = step_rows(&h.projects, "t-diamond").await;
    let by_name = |n: &str| {
        steps
            .iter()
            .find(|(s, _)| s == n)
            .unwrap_or_else(|| panic!("no step row {n}"))
            .1
            .clone()
    };
    let (a, b, c, d) = (by_name("a"), by_name("b"), by_name("c"), by_name("d"));
    assert!(a.finished_at.unwrap() <= b.started_at, "b waits for a");
    assert!(a.finished_at.unwrap() <= c.started_at, "c waits for a");
    assert!(b.finished_at.unwrap() <= d.started_at, "d waits for b");
    assert!(c.finished_at.unwrap() <= d.started_at, "d waits for c");
    assert_eq!(
        h.projects
            .get_todo("t-diamond")
            .await
            .unwrap()
            .unwrap()
            .status,
        ProjectTodoStatus::Done
    );
}

#[tokio::test]
async fn playbook_brain_step_routes_via_handoff_without_runtime() {
    // brain 钉住能力的步骤：resolve_step 以「钉住 + 缺省路由表」纯解析
    // （无需 brain 运行时），派发交接把解析结果带进子驱动；子 run 行按
    // 解析后的具体执行器（Agent/act）留痕并携带 capability 溯源。
    let h = harness_with(vec![done("brain done")]).await;
    seed_playbook(
        &h,
        &spec_of(
            "pb-brain",
            vec![PlaybookStep {
                name: "cap".into(),
                depends_on: vec![],
                target: PlaybookTarget::Brain {
                    capability_id: "cap-7".into(),
                },
                prompt: "用能力处理 {situation}".into(),
            }],
        ),
    )
    .await;
    seed_todo(&h, "t-brain", "pb-brain").await;

    let run_id = h.service.start_execute("t-brain").await.unwrap();
    let run = wait_run_done(&h.projects, &run_id).await;

    assert_eq!(run.status, ProjectTodoRunStatus::Done);
    let steps = step_rows(&h.projects, "t-brain").await;
    assert_eq!(steps.len(), 1);
    let child = &steps[0].1;
    // 子行按「解析后的执行器」留痕：Agent（缺省路由），不是 Brain/Playbook。
    assert_eq!(child.executor_kind, ProjectExecutorKind::Agent);
    assert_eq!(child.capability_id.as_deref(), Some("cap-7"));
    assert_eq!(child.plan_id, None);
    // {situation} 占位符已渲染（父 todo 的草稿进入步骤指令）。
    let snap: serde_json::Value =
        serde_json::from_str(child.input_snapshot.as_deref().unwrap()).unwrap();
    assert!(snap["prompt"].as_str().unwrap().contains("整理项目结构"));
}
