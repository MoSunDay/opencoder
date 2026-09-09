use super::*;

#[tokio::test]
async fn reserved_project_waits_without_stale_convergence_and_excludes_another_plan() {
    let (_dir, deps, store) = test_deps().await;
    seed_todo(&store, "queued", ProjectTodoStatus::Draft).await;
    seed_run(&store, "queued-run", "queued", ProjectTodoRunKind::Plan).await;
    deps.reserved.lock().unwrap().insert("queued-run".into());
    assert_eq!(recover::sweep_stale_runs(&deps, 0).await, 0);
    assert!(ensure_no_plan_in_flight(&deps, "queued").await.is_err());
    assert_eq!(
        store
            .get_todo_run("queued-run")
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Running
    );
    deps.reserved.lock().unwrap().remove("queued-run");
    assert_eq!(recover::sweep_stale_runs(&deps, 0).await, 1);
    assert_eq!(
        store
            .get_todo_run("queued-run")
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Failed
    );
}

#[tokio::test]
async fn panic_convergence_after_run_done_still_fails_stuck_running_todo() {
    let (_dir, deps, p) = test_deps().await;
    // 「close_run(Done) 与 todo 回写之间 panic」形状：run 已 Done、todo
    // 悬在 Running——run 标签不动，todo 必须补收敛为 Failed。
    seed_todo(&p, "t1", ProjectTodoStatus::Running).await;
    seed_run(&p, "r1", "t1", ProjectTodoRunKind::Execute).await;
    p.patch_todo_run(
        "r1",
        &ProjectTodoRunPatch {
            status: Some(RunStatus::Done),
            output_md: Some("执行完成".into()),
            finished_at: Some(2000),
            ..Default::default()
        },
        2000,
    )
    .await
    .unwrap();

    recover::converge_panicked_run(&deps, "r1", "t1", ProjectTodoRunKind::Execute).await;

    assert_eq!(run_status(&p, "r1").await, RunStatus::Done);
    assert_eq!(
        todo_status(&p, "t1").await,
        ProjectTodoStatus::Failed,
        "run 已终态但 todo 仍 Running：必须补收敛，不悬死"
    );
}

#[tokio::test]
async fn start_execute_blocks_unregistered_plan_run_within_grace() {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let service = ProjectService::new();
    service
        .init(
            store.clone(),
            store.clone(),
            dir.path().to_path_buf(),
            None,
            None,
        )
        .await
        .unwrap();
    seed_todo(&store, "t1", ProjectTodoStatus::Planned).await;
    // 未注册但刚起步：可能是并发 start_plan 的 create→注册窗口，保守拒绝。
    seed_run_at(
        &store,
        "r-fresh",
        "t1",
        ProjectTodoRunKind::Plan,
        opencoder_core::message::now_ms(),
    )
    .await;

    let err = service.start_execute("t1").await.unwrap_err();
    assert!(err.to_string().contains("plan"), "got: {err:#}");
}

#[tokio::test]
async fn start_execute_converges_stale_plan_run_past_grace() {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let service = ProjectService::new();
    service
        .init(
            store.clone(),
            store.clone(),
            dir.path().to_path_buf(),
            None,
            None,
        )
        .await
        .unwrap();
    seed_todo(&store, "t1", ProjectTodoStatus::Planned).await;
    // 崩溃残留：不在注册表且超 grace 的 running plan 行——不阻塞执行，
    // 机会式收敛为 Failed（sweep 同款文案）。
    seed_run_at(
        &store,
        "r-stale",
        "t1",
        ProjectTodoRunKind::Plan,
        opencoder_core::message::now_ms() - STALE_RUN_GRACE_MS - 1,
    )
    .await;

    let run_id = service.start_execute("t1").await.unwrap();
    assert_ne!(run_id, "r-stale", "新 execute run，而非残留 plan 行");
    let stale = store.get_todo_run("r-stale").await.unwrap().unwrap();
    assert_eq!(stale.status, RunStatus::Failed);
    assert_eq!(
        stale.output_md.as_deref(),
        Some("stale run converged: driver lost (restart/panic)")
    );
}

#[tokio::test]
async fn cancel_converges_lost_driver_execute_run_to_cancelled() {
    let (_dir, deps, p) = test_deps().await;
    seed_todo(&p, "t1", ProjectTodoStatus::Running).await;
    seed_run(&p, "r1", "t1", ProjectTodoRunKind::Execute).await;
    // 服务持独立（空）注册表：r1 不在其中 = 驱动已丢失。
    let service = ProjectService::new();
    service
        .init(
            deps.store.clone(),
            deps.projects.clone(),
            deps.workdir.clone(),
            None,
            None,
        )
        .await
        .unwrap();

    assert!(service.cancel("r1").await.unwrap());

    let run = p.get_todo_run("r1").await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Cancelled);
    assert!(run.finished_at.is_some(), "converge 落 finished_at");
    assert_eq!(
        todo_status(&p, "t1").await,
        ProjectTodoStatus::Planned,
        "lost-driver 取消回退 Planned（方案仍在，可再次执行）"
    );
}

#[tokio::test]
async fn cancel_terminal_or_missing_run_stays_false() {
    let (_dir, deps, p) = test_deps().await;
    seed_todo(&p, "t1", ProjectTodoStatus::Done).await;
    seed_run(&p, "r1", "t1", ProjectTodoRunKind::Execute).await;
    p.patch_todo_run(
        "r1",
        &ProjectTodoRunPatch {
            status: Some(RunStatus::Done),
            finished_at: Some(2000),
            ..Default::default()
        },
        2000,
    )
    .await
    .unwrap();
    let service = ProjectService::new();
    service
        .init(
            deps.store.clone(),
            deps.projects.clone(),
            deps.workdir.clone(),
            None,
            None,
        )
        .await
        .unwrap();

    assert!(!service.cancel("r1").await.unwrap(), "已终态：无可取消");
    assert!(
        !service.cancel("missing").await.unwrap(),
        "行缺失：无可取消"
    );
    assert_eq!(run_status(&p, "r1").await, RunStatus::Done, "终态不被打花");
}

#[tokio::test]
async fn repeated_cancel_does_not_converge_a_driver_still_flushing_output() {
    let (_dir, deps, store) = test_deps().await;
    seed_todo(&store, "t1", ProjectTodoStatus::Running).await;
    seed_run(&store, "r1", "t1", ProjectTodoRunKind::Execute).await;
    let service = ProjectService::new();
    service
        .init(
            deps.store.clone(),
            deps.projects.clone(),
            deps.workdir.clone(),
            None,
            None,
        )
        .await
        .unwrap();
    let live = service.require().unwrap();
    let token = spawn_run(&live, "r1");

    assert!(service.cancel("r1").await.unwrap());
    assert!(token.is_cancelled());
    assert!(service.cancel("r1").await.unwrap());
    assert_eq!(crate::recover::sweep_stale_runs(&live, 0).await, 0);
    let run = store.get_todo_run("r1").await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.output_md, None);
    assert_eq!(todo_status(&store, "t1").await, ProjectTodoStatus::Running);

    crate::plan_gen::close_run(
        &live,
        "r1",
        RunStatus::Cancelled,
        Some("partial output before cancellation".into()),
        None,
        None,
    )
    .await;
    crate::plan_gen::forget_spawn(&live, "r1");
    let run = store.get_todo_run("r1").await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Cancelled);
    assert_eq!(
        run.output_md.as_deref(),
        Some("partial output before cancellation")
    );
    assert!(!service.cancel("r1").await.unwrap());
}

// ---- run_agent_label：解析出的 agent 名优先于 todo.agent（D3②） ----

fn label_todo() -> ProjectTodoRecord {
    ProjectTodoRecord {
        id: "t1".into(),
        milestone_id: None,
        title: "待办".into(),
        draft: "草稿".into(),
        plan_md: Some("# 方案".into()),
        status: ProjectTodoStatus::Planned,
        agent: "act".into(),
        executor_kind: ProjectExecutorKind::Brain,
        executor_ref: Some("cap-1".into()),
        executor_spec: None,
        active_session_id: None,
        created_at: 1,
        updated_at: 1,
    }
}

/// agent 目标带名（brain 路由 / 控制面 override）→ 标签用解析出的代理名；
/// 不带名（普通 agent todo 的 resolve 产出）→ 沿用 todo.agent；team/dag
/// 前缀不受影响。
#[test]
fn run_agent_label_prefers_resolved_ref_over_todo_agent() {
    let todo = label_todo();
    assert_eq!(
        run_agent_label(
            &ResolvedExecutor {
                kind: ProjectExecutorKind::Agent,
                ref_: Some("explore".into()),
            },
            &todo
        ),
        "explore",
        "brain 路由/override 带名：标签是实际代理名"
    );
    assert_eq!(
        run_agent_label(
            &ResolvedExecutor {
                kind: ProjectExecutorKind::Agent,
                ref_: None,
            },
            &todo
        ),
        "act",
        "普通 agent 直驱：沿用 todo.agent"
    );
    assert_eq!(
        run_agent_label(
            &ResolvedExecutor {
                kind: ProjectExecutorKind::Team,
                ref_: Some("fleet".into()),
            },
            &todo
        ),
        "team:fleet"
    );
    assert_eq!(
        run_agent_label(
            &ResolvedExecutor {
                kind: ProjectExecutorKind::Dag,
                ref_: Some("dag-def".into()),
            },
            &todo
        ),
        "dag:dag-def"
    );
}
