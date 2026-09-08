//! `service.rs` 的单元测试：stale run 清扫、取消收敛、plan 互斥窗口与
//! panic 兜底（手工构造 Deps，内存库）。从 service.rs 拆出以遵守单文件
//! 行数上限。

use super::*;
use opencoder_store::{
    LibsqlStore, ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunPatch, ProjectTodoRunRecord,
    ProjectTodoRunStatus as RunStatus, ProjectTodoStatus,
};

use crate::recover;

#[tokio::test]
async fn uninitialized_require_and_read_paths_error_cleanly() {
    let service = ProjectService::new();
    assert!(service.start_plan("t1").await.is_err());
    assert!(service.overview().await.is_err());
}

#[tokio::test]
async fn cancel_unknown_or_uninitialized_returns_false() {
    let service = ProjectService::new();
    assert!(!service.cancel("nope").await.unwrap());
}

// ---- recover: panic 兜底 + stale 清扫（手工构造 Deps，内存库） ----

async fn test_deps() -> (tempfile::TempDir, Arc<Deps>, Arc<LibsqlStore>) {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let deps = Arc::new(Deps {
        store: store.clone(),
        projects: store.clone(),
        workdir: dir.path().to_path_buf(),
        client_override: None,
        brain: None,
        spawns: Mutex::new(HashMap::new()),
        archive_root: Mutex::new(dir.path().join("runs")),
        admission: tokio::sync::Mutex::new(()),
        persistence_error: Mutex::new(None),
    });
    (dir, deps, store)
}

async fn seed_todo(p: &Arc<LibsqlStore>, id: &str, status: ProjectTodoStatus) {
    let now = 1000;
    p.create_todo(&ProjectTodoRecord {
        id: id.into(),
        milestone_id: None,
        title: format!("待办 {id}"),
        draft: "草稿".into(),
        plan_md: Some("# 方案".into()),
        status,
        agent: "act".into(),
        executor_kind: opencoder_store::ProjectExecutorKind::Agent,
        executor_ref: None,
        executor_spec: None,
        active_session_id: None,
        created_at: now,
        updated_at: now,
    })
    .await
    .unwrap();
}

async fn seed_run(p: &Arc<LibsqlStore>, id: &str, todo_id: &str, kind: ProjectTodoRunKind) {
    seed_run_at(p, id, todo_id, kind, 1000).await;
}

async fn seed_run_at(
    p: &Arc<LibsqlStore>,
    id: &str,
    todo_id: &str,
    kind: ProjectTodoRunKind,
    started_at: i64,
) {
    let version = p.next_todo_version(todo_id).await.unwrap();
    p.create_todo_run(&ProjectTodoRunRecord {
        input_snapshot: None,
        trace_manifest: None,
        id: id.into(),
        todo_id: todo_id.into(),
        kind,
        version,
        plan_md: None,
        output_md: None,
        agent: "act".into(),
        executor_kind: opencoder_store::ProjectExecutorKind::Agent,
        capability_id: None,
        plan_id: None,
        output_ref: None,
        session_id: None,
        status: RunStatus::Running,
        started_at,
        finished_at: None,
        created_at: started_at,
    })
    .await
    .unwrap();
}

async fn todo_status(p: &Arc<LibsqlStore>, id: &str) -> ProjectTodoStatus {
    p.get_todo(id).await.unwrap().unwrap().status
}

async fn run_status(p: &Arc<LibsqlStore>, id: &str) -> RunStatus {
    p.get_todo_run(id).await.unwrap().unwrap().status
}

#[tokio::test]
async fn panic_convergence_fails_run_execute_todo_and_forgets_spawn() {
    let (_dir, deps, p) = test_deps().await;
    seed_todo(&p, "t1", ProjectTodoStatus::Running).await;
    seed_run(&p, "r1", "t1", ProjectTodoRunKind::Execute).await;
    deps.spawns
        .lock()
        .unwrap()
        .insert("r1".into(), CancellationToken::new());

    recover::converge_panicked_run(&deps, "r1", "t1", ProjectTodoRunKind::Execute).await;

    let run = p.get_todo_run("r1").await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.output_md.as_deref(), Some("run driver panicked"));
    assert!(run.finished_at.is_some(), "close_run stamps finished_at");
    assert_eq!(todo_status(&p, "t1").await, ProjectTodoStatus::Failed);
    assert!(
        !deps.spawns.lock().unwrap().contains_key("r1"),
        "panic convergence removes the spawn token"
    );
}

#[tokio::test]
async fn panic_convergence_leaves_plan_todo_untouched() {
    let (_dir, deps, p) = test_deps().await;
    seed_todo(&p, "t1", ProjectTodoStatus::Planned).await;
    seed_run(&p, "r1", "t1", ProjectTodoRunKind::Plan).await;

    recover::converge_panicked_run(&deps, "r1", "t1", ProjectTodoRunKind::Plan).await;

    assert_eq!(run_status(&p, "r1").await, RunStatus::Failed);
    assert_eq!(
        todo_status(&p, "t1").await,
        ProjectTodoStatus::Planned,
        "plan runs do not own the todo status"
    );
}

#[tokio::test]
async fn sweep_converges_only_unregistered_runs_past_grace() {
    let (_dir, deps, p) = test_deps().await;
    // 三个 execute/plan 各一：过期未注册（要收敛）、刚启动未注册（宽限）、
    // 过期但已注册（本进程驱动仍在跑）。另有一个过期 run 挂在非 Running
    // 的 todo 上：run 收敛但 todo 不被动。
    seed_todo(&p, "t-exec", ProjectTodoStatus::Running).await;
    seed_todo(&p, "t-plan", ProjectTodoStatus::Planned).await;
    seed_todo(&p, "t-live", ProjectTodoStatus::Running).await;
    seed_todo(&p, "t-done", ProjectTodoStatus::Done).await;
    seed_run(&p, "r-exec", "t-exec", ProjectTodoRunKind::Execute).await;
    seed_run(&p, "r-plan", "t-plan", ProjectTodoRunKind::Plan).await;
    seed_run(&p, "r-live", "t-live", ProjectTodoRunKind::Execute).await;
    seed_run(&p, "r-done", "t-done", ProjectTodoRunKind::Execute).await;
    // 「刚刚启动」的未注册 running run：宽限期内不动（正常启动到
    // spawn 注册之间存在毫秒级窗口，靠 grace 兜住）。
    let fresh_started = opencoder_core::message::now_ms();
    seed_run_at(
        &p,
        "r-fresh",
        "t-exec",
        ProjectTodoRunKind::Execute,
        fresh_started,
    )
    .await;
    deps.spawns
        .lock()
        .unwrap()
        .insert("r-live".into(), CancellationToken::new());

    let converged = recover::sweep_stale_runs(&deps, 300_000).await;
    assert_eq!(converged, 3, "r-exec + r-plan + r-done converge");

    let exec = p.get_todo_run("r-exec").await.unwrap().unwrap();
    assert_eq!(exec.status, RunStatus::Failed);
    assert_eq!(
        exec.output_md.as_deref(),
        Some("stale run converged: driver lost (restart/panic)")
    );
    assert_eq!(todo_status(&p, "t-exec").await, ProjectTodoStatus::Failed);
    assert_eq!(run_status(&p, "r-plan").await, RunStatus::Failed);
    assert_eq!(
        todo_status(&p, "t-plan").await,
        ProjectTodoStatus::Planned,
        "plan run convergence never touches the todo"
    );
    assert_eq!(
        todo_status(&p, "t-done").await,
        ProjectTodoStatus::Done,
        "non-running todo stays as-is even for execute runs"
    );
    assert_eq!(
        run_status(&p, "r-fresh").await,
        RunStatus::Running,
        "inside the grace window: kept"
    );
    assert_eq!(
        run_status(&p, "r-live").await,
        RunStatus::Running,
        "registered token: this process still owns the driver"
    );
    assert_eq!(todo_status(&p, "t-live").await, ProjectTodoStatus::Running);
}

#[tokio::test]
async fn start_execute_rejects_while_plan_run_in_flight() {
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
    seed_run(&store, "r-plan", "t1", ProjectTodoRunKind::Plan).await;
    // 真活驱动：令牌在注册表里（seed 的 started_at 很老，只有注册表
    // 命中才能证明「进行中」）。
    service
        .deps
        .get()
        .unwrap()
        .spawns
        .lock()
        .unwrap()
        .insert("r-plan".into(), CancellationToken::new());

    let err = service.start_execute("t1").await.unwrap_err();
    assert!(err.to_string().contains("plan"), "got: {err:#}");
    assert_eq!(
        todo_status(&store, "t1").await,
        ProjectTodoStatus::Planned,
        "plan run 进行中不拿走 execute claim"
    );
}

#[tokio::test]
async fn panic_convergence_keeps_terminal_run_label() {
    let (_dir, deps, p) = test_deps().await;
    seed_todo(&p, "t1", ProjectTodoStatus::Done).await;
    seed_run(&p, "r1", "t1", ProjectTodoRunKind::Execute).await;
    // 驱动已在 panic 前把 run 收敛到 Done 并回写 todo（Done）。
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
    deps.spawns
        .lock()
        .unwrap()
        .insert("r1".into(), CancellationToken::new());

    recover::converge_panicked_run(&deps, "r1", "t1", ProjectTodoRunKind::Execute).await;

    let run = p.get_todo_run("r1").await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Done, "终态标签不被兜底改写");
    assert_eq!(
        run.output_md.as_deref(),
        Some("执行完成"),
        "原始输出不被 \"run driver panicked\" 打花"
    );
    assert_eq!(todo_status(&p, "t1").await, ProjectTodoStatus::Done);
    assert!(
        !deps.spawns.lock().unwrap().contains_key("r1"),
        "panic convergence removes the spawn token"
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
