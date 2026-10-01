use super::*;

#[tokio::test]
async fn plan_generates_and_updates_todo() {
    let h = harness(vec![done("# 实施计划\n1. 步骤一")]).await;
    seed_goal(&h.projects, "g1").await;
    seed_initiative(&h.projects, "m1", "g1").await;
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
async fn standalone_initiative_can_plan_and_execute_without_a_project() {
    let h = harness(vec![done("专项实施计划"), done("专项完成")]).await;
    let now = 1000;
    h.projects
        .create_initiative(&ProjectInitiativeRecord {
            id: "initiative".into(),
            goal_id: None,
            title: "专项治理".into(),
            detail_md: Some("专项背景".into()),
            status: ProjectInitiativeStatus::Planned,
            sort: 0,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let todo = seed_todo(&h.projects, "initiative-todo", Some("initiative")).await;
    let plan = h.service.start_plan(&todo).await.unwrap();
    let plan = wait_run_done(&h.projects, &plan).await;
    assert_eq!(plan.status, ProjectTodoRunStatus::Done);
    let input: serde_json::Value =
        serde_json::from_str(plan.input_snapshot.as_ref().unwrap()).unwrap();
    let prompt = input["prompt"].as_str().unwrap();
    assert!(prompt.contains("专项治理"));
    assert!(prompt.contains("专项背景"));
    assert!(!prompt.contains("- 目标："));
    let execution = h.service.start_execute(&todo).await.unwrap();
    assert_eq!(
        wait_run_done(&h.projects, &execution).await.status,
        ProjectTodoRunStatus::Done
    );
    let overview = h.service.overview().await.unwrap();
    assert!(overview["goals"].as_array().unwrap().is_empty());
    assert_eq!(
        overview["standalone_initiatives"][0]["todos"][0]["status"],
        "done"
    );
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
async fn execute_run_snapshots_plan_md_at_start() {
    let h = harness(vec![done("执行完成")]).await;
    let todo_id = seed_todo(&h.projects, "t1", None).await;
    h.projects
        .patch_todo(
            &todo_id,
            &ProjectTodoPatch {
                plan_md: Some(Some("# 方案快照\n1. 步骤一".into())),
                status: Some(ProjectTodoStatus::Planned),
                ..Default::default()
            },
            2000,
        )
        .await
        .unwrap();

    let run_id = h.service.start_execute(&todo_id).await.unwrap();
    // 启动即快照：run 行携带执行起点的方案正文，与 todo.plan_md 一致。
    let run = h.projects.get_todo_run(&run_id).await.unwrap().unwrap();
    assert_eq!(run.kind, ProjectTodoRunKind::Execute);
    assert_eq!(run.plan_md.as_deref(), Some("# 方案快照\n1. 步骤一"));
    let todo = h.projects.get_todo(&todo_id).await.unwrap().unwrap();
    assert_eq!(todo.plan_md, run.plan_md);
}

#[tokio::test]
async fn overview_tree_shape() {
    let h = harness(vec![]).await;
    seed_goal(&h.projects, "g1").await;
    seed_initiative(&h.projects, "m1", "g1").await;
    seed_todo(&h.projects, "t-in", Some("m1")).await;
    seed_todo(&h.projects, "t-backlog", None).await;

    let tree = h.service.overview().await.unwrap();
    assert_eq!(tree["goals"].as_array().unwrap().len(), 1);
    let goal = &tree["goals"][0];
    assert_eq!(goal["id"], "g1");
    let initiatives = goal["initiatives"].as_array().unwrap();
    assert_eq!(initiatives.len(), 1);
    assert_eq!(initiatives[0]["id"], "m1");
    let todos = initiatives[0]["todos"].as_array().unwrap();
    assert_eq!(todos.len(), 1);
    assert_eq!(todos[0]["id"], "t-in");
    let backlog = tree["backlog"].as_array().unwrap();
    assert_eq!(backlog.len(), 1);
    assert_eq!(backlog[0]["id"], "t-backlog");
}

#[tokio::test]
async fn execute_proceeds_after_stale_plan_run_is_converged() {
    let h = harness(vec![done("执行完成")]).await;
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
    // 崩溃残留：plan run 行 running、不在注册表、超 grace（5 分钟）——
    // 不再阻塞执行，机会式收敛为 Failed 后放行。
    let stale_started = opencoder_core::message::now_ms() - 301_000;
    h.projects
        .create_todo_run(&ProjectTodoRunRecord {
            input_snapshot: None,
            trace_manifest: None,
            id: "prun-stale".into(),
            todo_id: todo_id.clone(),
            kind: ProjectTodoRunKind::Plan,
            version: 1,
            plan_md: None,
            output_md: None,
            agent: "plan".into(),
            executor_kind: ProjectExecutorKind::Agent,
            capability_id: None,
            plan_id: None,
            output_ref: None,
            session_id: None,
            status: ProjectTodoRunStatus::Running,
            started_at: stale_started,
            finished_at: None,
            created_at: stale_started,
        })
        .await
        .unwrap();

    let run_id = h.service.start_execute(&todo_id).await.unwrap();
    let run = wait_run_done(&h.projects, &run_id).await;
    assert_eq!(run.status, ProjectTodoRunStatus::Done);
    assert_eq!(run.output_md.as_deref(), Some("执行完成"));
    let stale = h
        .projects
        .get_todo_run("prun-stale")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stale.status, ProjectTodoRunStatus::Failed);
    assert_eq!(
        stale.output_md.as_deref(),
        Some("stale run converged: driver lost (restart/panic)")
    );
    let todo = h.projects.get_todo(&todo_id).await.unwrap().unwrap();
    assert_eq!(todo.status, ProjectTodoStatus::Done);
}

#[tokio::test]
async fn atomic_claim_failure_leaves_todo_and_runs_untouched() {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let service = ProjectService::new();
    service
        .init(
            store.clone(),
            Arc::new(CreateRunFailingStore {
                inner: store.clone(),
            }),
            dir.path().to_path_buf(),
            None,
            None,
        )
        .await
        .unwrap();
    let todo_id = seed_todo(&(store.clone() as Arc<dyn ProjectStore>), "t1", None).await;
    store
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

    let err = service.start_execute(&todo_id).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("claim todo and create execute run"),
        "got: {err:#}"
    );

    // 原子入口失败：todo 保持 Planned，且没有 execute run 行残留。
    let todo = store.get_todo(&todo_id).await.unwrap().unwrap();
    assert_eq!(todo.status, ProjectTodoStatus::Planned);
    let runs = store.list_todo_runs(&todo_id).await.unwrap();
    assert!(
        runs.iter().all(|r| r.kind != ProjectTodoRunKind::Execute),
        "no execute run row should exist, got {runs:?} kinds"
    );
}
