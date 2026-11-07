//! Terminal run write-backs by kind (Plan/Execute/Step) and execution links.
use super::*;
use crate::suite_runs::seed_running_run;

/// Plan 完成：todo 回 Planned，输出写进 plan_md（方案轨的落盘点）。
#[tokio::test]
async fn plan_finish_writes_back_planned_and_plan_md() {
    let p = mem_iface().await;
    p.create_todo(&todo("t1", None, 1)).await.unwrap();
    seed_running_run(p.as_ref(), "t1", "r-plan", 20, ProjectTodoRunKind::Plan).await;

    assert!(p
        .finish_todo_run(
            "r-plan",
            &ProjectTodoRunPatch {
                output_md: Some("# 新方案".into()),
                ..rpatch(Some(ProjectTodoRunStatus::Done), Some(21))
            },
            21,
        )
        .await
        .unwrap());
    let t1 = p.get_todo("t1").await.unwrap().unwrap();
    assert_eq!(t1.status, ProjectTodoStatus::Planned);
    assert_eq!(t1.plan_md.as_deref(), Some("# 新方案"));
    assert_eq!(t1.updated_at, 21);
    let run = p.get_todo_run("r-plan").await.unwrap().unwrap();
    assert_eq!(run.status, ProjectTodoRunStatus::Done);
    assert_eq!(run.finished_at, Some(21));
    assert_eq!(run.output_md.as_deref(), Some("# 新方案"));

    // Terminal replay: already Done, so a second finish is a no-op false
    // and does not relabel the todo.
    assert!(!p
        .finish_todo_run(
            "r-plan",
            &rpatch(Some(ProjectTodoRunStatus::Failed), Some(22)),
            22
        )
        .await
        .unwrap());
    assert_eq!(
        p.get_todo("t1").await.unwrap().unwrap().status,
        ProjectTodoStatus::Planned
    );
}

/// Execute 完成：todo → Done；失败 → Failed（执行轨独占 todo 终态）。
#[tokio::test]
async fn execute_finish_owns_the_todo_terminal_status() {
    let p = mem_iface().await;
    p.create_todo(&todo("t2", None, 2)).await.unwrap();
    seed_running_run(p.as_ref(), "t2", "r-exec", 20, ProjectTodoRunKind::Execute).await;

    assert!(p
        .finish_todo_run(
            "r-exec",
            &ProjectTodoRunPatch {
                output_md: Some("完成".into()),
                ..rpatch(Some(ProjectTodoRunStatus::Done), Some(21))
            },
            21,
        )
        .await
        .unwrap());
    assert_eq!(
        p.get_todo("t2").await.unwrap().unwrap().status,
        ProjectTodoStatus::Done
    );

    // Terminal replay with a status: no second write-back (Ok(false)), and
    // a missing terminal status on a still-RUNNING row is a contract
    // violation ("final run status missing").
    assert!(!p
        .finish_todo_run(
            "r-exec",
            &rpatch(Some(ProjectTodoRunStatus::Failed), Some(22)),
            22
        )
        .await
        .unwrap());
    assert_eq!(
        p.get_todo("t2").await.unwrap().unwrap().status,
        ProjectTodoStatus::Done
    );
    p.create_todo(&todo("t2d", None, 5)).await.unwrap();
    seed_running_run(
        p.as_ref(),
        "t2d",
        "r-exec4",
        50,
        ProjectTodoRunKind::Execute,
    )
    .await;
    assert!(p
        .finish_todo_run("r-exec4", &rpatch(None, None), 51)
        .await
        .is_err());
    assert_eq!(
        p.get_todo_run("r-exec4").await.unwrap().unwrap().status,
        ProjectTodoRunStatus::Running
    );

    // Failed convergence on a fresh execute run flips the todo to Failed.
    p.create_todo(&todo("t2b", None, 3)).await.unwrap();
    seed_running_run(
        p.as_ref(),
        "t2b",
        "r-exec2",
        30,
        ProjectTodoRunKind::Execute,
    )
    .await;
    assert!(p
        .finish_todo_run(
            "r-exec2",
            &ProjectTodoRunPatch {
                output_md: Some("boom".into()),
                ..rpatch(Some(ProjectTodoRunStatus::Failed), Some(31))
            },
            31,
        )
        .await
        .unwrap());
    let t2b = p.get_todo("t2b").await.unwrap().unwrap();
    assert_eq!(t2b.status, ProjectTodoStatus::Failed);
    // Cancelled rolls the todo back to Planned (retryable from the plan).
    p.create_todo(&todo("t2c", None, 4)).await.unwrap();
    seed_running_run(
        p.as_ref(),
        "t2c",
        "r-exec3",
        40,
        ProjectTodoRunKind::Execute,
    )
    .await;
    assert!(p
        .finish_todo_run(
            "r-exec3",
            &rpatch(Some(ProjectTodoRunStatus::Cancelled), Some(41)),
            41
        )
        .await
        .unwrap());
    assert_eq!(
        p.get_todo("t2c").await.unwrap().unwrap().status,
        ProjectTodoStatus::Planned
    );
}

/// Step 完成：run 行自身收敛，todo 保持 Running；重复终结返回 false。
#[tokio::test]
async fn step_finish_never_touches_the_todo() {
    let p = mem_iface().await;
    p.create_todo(&todo("t3", None, 3)).await.unwrap();
    seed_running_run(p.as_ref(), "t3", "r-step", 20, ProjectTodoRunKind::Step).await;

    assert!(p
        .finish_todo_run(
            "r-step",
            &ProjectTodoRunPatch {
                output_md: Some("步骤产出".into()),
                ..rpatch(Some(ProjectTodoRunStatus::Done), Some(21))
            },
            21,
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
        t3.updated_at, 21,
        "a Step finish must not re-stamp the todo"
    );

    // 已终结的行重复终结：false，状态不再翻转（也不会把 todo 改 Failed）。
    assert!(!p
        .finish_todo_run(
            "r-step",
            &rpatch(Some(ProjectTodoRunStatus::Failed), Some(22)),
            22
        )
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

/// Run kind data lives in run rows and survives reopen; the finish path
/// guards on the row it actually sees.
#[tokio::test]
async fn finish_rejects_missing_rows_and_non_terminal_patches() {
    let p = mem_iface().await;
    p.create_todo(&todo("t4", None, 4)).await.unwrap();
    // Missing run row is a bug-shaped error, not Ok(false).
    assert!(p
        .finish_todo_run(
            "nope",
            &rpatch(Some(ProjectTodoRunStatus::Done), Some(1)),
            1
        )
        .await
        .is_err());
    seed_running_run(p.as_ref(), "t4", "r4", 10, ProjectTodoRunKind::Execute).await;
    // Running is not a terminal status to finalize with.
    assert!(p
        .finish_todo_run("r4", &rpatch(Some(ProjectTodoRunStatus::Running), None), 11)
        .await
        .is_err());
}

#[tokio::test]
async fn todo_assignments_only_keep_references_and_cascade_links_on_delete() {
    let p = mem_iface().await;
    p.create_todo(&todo("linked", None, 1)).await.unwrap();
    for (id, kind) in [
        ("agent-a", "agent"),
        ("agent-a", "agent"),
        ("brain-b", "brain"),
    ] {
        p.link_todo_execution(&opencoder_store::project::ProjectAssignment {
            todo_id: "linked".into(),
            execution_id: id.into(),
            kind: kind.into(),
            name: id.into(),
            created_at: 10,
            capability_id: Some(format!("capability-{kind}")),
        })
        .await
        .unwrap();
    }
    let rows = p.list_todo_assignments("linked").await.unwrap();
    assert_eq!(rows.len(), 2, "INSERT-OR-IGNORE by (todo, execution)");
    assert_eq!(rows[0].kind, "brain");
    assert_eq!(rows[0].capability_id.as_deref(), Some("capability-brain"));
    let latest = p.latest_todo_assignments().await.unwrap();
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].execution_id, "brain-b");
    assert!(p.unlink_todo_execution("linked", "brain-b").await.unwrap());
    assert!(!p.unlink_todo_execution("linked", "brain-b").await.unwrap());
    assert_eq!(p.list_todo_assignments("linked").await.unwrap().len(), 1);
    p.delete_todo("linked").await.unwrap();
    assert!(p.list_todo_assignments("linked").await.unwrap().is_empty());
    assert!(p.latest_todo_assignments().await.unwrap().is_empty());
}
