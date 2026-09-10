//! `finish_todo_run` 的终态回写语义（libsql 落地）：按 run kind 决定 todo
//! 状态的归属——Plan 回 Planned（plan_md 落盘）、Execute 独占 todo 终态、
//! Step（playbook 子尝试）永不回写 todo 状态，todo 生命周期由父
//! playbook 的 Execute 行独占。幂等守卫（重复终结返回 false）一并覆盖。

use std::sync::Arc;

use opencoder_store::{
    LibsqlStore, ProjectExecutorKind, ProjectStore, ProjectTodoPatch, ProjectTodoRecord,
    ProjectTodoRunKind, ProjectTodoRunPatch, ProjectTodoRunRecord, ProjectTodoRunStatus,
    ProjectTodoStatus,
};

async fn fresh() -> Arc<dyn ProjectStore> {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(LibsqlStore::open(dir.path().join("test.db")).await.unwrap());
    store
}

fn todo(id: &str, created_at: i64) -> ProjectTodoRecord {
    ProjectTodoRecord {
        id: id.to_string(),
        milestone_id: None,
        title: format!("todo {id}"),
        draft: format!("draft {id}"),
        plan_md: None,
        status: ProjectTodoStatus::Draft,
        agent: "act".to_string(),
        executor_kind: ProjectExecutorKind::Agent,
        executor_ref: None,
        executor_spec: None,
        active_session_id: None,
        created_at,
        updated_at: created_at,
    }
}

/// 落一条指定 kind 的 Running run 行（不 claim——Step 行从不 claim）。
async fn run_of(p: &dyn ProjectStore, id: &str, todo_id: &str, kind: ProjectTodoRunKind) {
    let version = p.next_todo_version(todo_id).await.unwrap();
    p.create_todo_run(&ProjectTodoRunRecord {
        input_snapshot: None,
        trace_manifest: None,
        id: id.to_string(),
        todo_id: todo_id.to_string(),
        kind,
        version,
        plan_md: None,
        output_md: None,
        agent: "act".to_string(),
        executor_kind: ProjectExecutorKind::Agent,
        capability_id: None,
        plan_id: None,
        output_ref: None,
        session_id: None,
        status: ProjectTodoRunStatus::Running,
        started_at: 20,
        finished_at: None,
        created_at: 20,
    })
    .await
    .unwrap();
}

fn terminal(status: ProjectTodoRunStatus, output: Option<&str>) -> ProjectTodoRunPatch {
    ProjectTodoRunPatch {
        status: Some(status),
        output_md: output.map(str::to_string),
        finished_at: Some(21),
        ..Default::default()
    }
}

/// Plan 完成：todo 回 Planned，输出写进 plan_md（方案轨的落盘点）。
#[tokio::test]
async fn plan_finish_writes_back_planned_and_plan_md() {
    let p = fresh().await;
    p.create_todo(&todo("t1", 1)).await.unwrap();
    p.patch_todo(
        "t1",
        &ProjectTodoPatch {
            status: Some(ProjectTodoStatus::Planned),
            ..Default::default()
        },
        10,
    )
    .await
    .unwrap();
    assert!(p.claim_todo_running("t1", 11).await.unwrap());
    run_of(p.as_ref(), "r-plan", "t1", ProjectTodoRunKind::Plan).await;

    assert!(p
        .finish_todo_run(
            "r-plan",
            &terminal(ProjectTodoRunStatus::Done, Some("# 新方案")),
            21
        )
        .await
        .unwrap());
    let t1 = p.get_todo("t1").await.unwrap().unwrap();
    assert_eq!(t1.status, ProjectTodoStatus::Planned);
    assert_eq!(t1.plan_md.as_deref(), Some("# 新方案"));
    // run 行自身同时收敛。
    let run = p.get_todo_run("r-plan").await.unwrap().unwrap();
    assert_eq!(run.status, ProjectTodoRunStatus::Done);
    assert_eq!(run.finished_at, Some(21));
}

/// Execute 完成：todo → Done；失败/取消同理映射（失败 → Failed）。
#[tokio::test]
async fn execute_finish_owns_the_todo_terminal_status() {
    let p = fresh().await;
    p.create_todo(&todo("t2", 2)).await.unwrap();
    p.patch_todo(
        "t2",
        &ProjectTodoPatch {
            status: Some(ProjectTodoStatus::Planned),
            ..Default::default()
        },
        10,
    )
    .await
    .unwrap();
    assert!(p.claim_todo_running("t2", 11).await.unwrap());
    run_of(p.as_ref(), "r-exec", "t2", ProjectTodoRunKind::Execute).await;

    assert!(p
        .finish_todo_run(
            "r-exec",
            &terminal(ProjectTodoRunStatus::Done, Some("完成")),
            21
        )
        .await
        .unwrap());
    assert_eq!(
        p.get_todo("t2").await.unwrap().unwrap().status,
        ProjectTodoStatus::Done
    );
}

/// Step 完成：run 行自身收敛，todo 保持 Running 且不重盖章——父 playbook
/// 的 Execute 行稍后独占终态回写；重复终结返回 false（幂等守卫）。
#[tokio::test]
async fn step_finish_never_touches_the_todo() {
    let p = fresh().await;
    p.create_todo(&todo("t3", 3)).await.unwrap();
    p.patch_todo(
        "t3",
        &ProjectTodoPatch {
            status: Some(ProjectTodoStatus::Planned),
            ..Default::default()
        },
        10,
    )
    .await
    .unwrap();
    assert!(p.claim_todo_running("t3", 11).await.unwrap());
    run_of(p.as_ref(), "r-step", "t3", ProjectTodoRunKind::Step).await;

    assert!(p
        .finish_todo_run(
            "r-step",
            &terminal(ProjectTodoRunStatus::Done, Some("步骤产出")),
            21
        )
        .await
        .unwrap());
    let step = p.get_todo_run("r-step").await.unwrap().unwrap();
    assert_eq!(step.status, ProjectTodoRunStatus::Done);
    assert_eq!(step.output_md.as_deref(), Some("步骤产出"));
    assert_eq!(step.finished_at, Some(21));
    let t3 = p.get_todo("t3").await.unwrap().unwrap();
    assert_eq!(
        t3.status,
        ProjectTodoStatus::Running,
        "a Step run must never write the todo status"
    );
    assert_eq!(
        t3.updated_at, 11,
        "a Step finish must not re-stamp the todo"
    );

    // 已终结的行重复终结：false，状态不再翻转（也不会把 todo 改 Failed）。
    assert!(!p
        .finish_todo_run("r-step", &terminal(ProjectTodoRunStatus::Failed, None), 22)
        .await
        .unwrap());
    assert_eq!(
        p.get_todo_run("r-step").await.unwrap().unwrap().status,
        ProjectTodoRunStatus::Done
    );
    assert_eq!(
        p.get_todo("t3").await.unwrap().unwrap().status,
        ProjectTodoStatus::Running
    );
}
