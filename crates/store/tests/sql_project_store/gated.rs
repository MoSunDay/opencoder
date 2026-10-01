use opencoder_core::{StorageBackend, StorageConfig};
use opencoder_store::sql_store;
use opencoder_store::{
    ProjectExecutorKind, ProjectGoalPatch, ProjectGoalRecord, ProjectGoalStatus,
    ProjectInitiativeRecord, ProjectInitiativeStatus, ProjectStore, ProjectTodoPatch,
    ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunRecord, ProjectTodoRunStatus,
    ProjectTodoStatus,
};
use std::future::Future;
use std::time::Duration;
async fn crud_contract(p: &dyn ProjectStore, uniq: &str, expect_name: &str) {
    assert_eq!(p.project_backend_name(), expect_name);
    let (goal, goal2, ms, todo) = (
        format!("goal-{uniq}"),
        format!("goal2-{uniq}"),
        format!("ms-{uniq}"),
        format!("todo-{uniq}"),
    );
    let ts = 1_000i64;
    goals::seed_goals(p, &goal, &goal2, ts).await;
    p.create_initiative(&ProjectInitiativeRecord {
        id: ms.clone(),
        goal_id: Some(goal.clone()),
        title: "ms".into(),
        detail_md: None,
        status: ProjectInitiativeStatus::Planned,
        sort: 1,
        created_at: ts + 3,
        updated_at: ts + 3,
    })
    .await
    .unwrap();
    let ms_ok = || async {
        let mss = p.list_initiatives(Some(&goal)).await.unwrap();
        mss.len() == 1 && mss[0].id == ms
    };
    eventually("milestone visible under goal", ms_ok).await;
    p.create_todo(&ProjectTodoRecord {
        id: todo.clone(),
        initiative_id: Some(ms.clone()),
        title: "todo".into(),
        draft: "draft".into(),
        plan_md: None,
        status: ProjectTodoStatus::Draft,
        agent: "act".into(),
        executor_kind: ProjectExecutorKind::Team,
        executor_ref: Some("feature-team".into()),
        executor_spec: Some("{\"members\":[]}".into()),
        active_session_id: None,
        board_status: "backlog".into(),
        position: 0,
        capability_id: None,
        created_at: ts + 4,
        updated_at: ts + 4,
    })
    .await
    .unwrap();
    let todo_ok = || async {
        let todos = p.list_todos(Some(&ms)).await.unwrap();
        todos.len() == 1
            && todos[0].id == todo
            && todos[0].executor_kind == ProjectExecutorKind::Team
            && todos[0].executor_ref.as_deref() == Some("feature-team")
            && todos[0].executor_spec.as_deref() == Some("{\"members\":[]}")
    };
    eventually("todo visible under milestone", todo_ok).await;
    let v1 = p.next_todo_version(&todo).await.unwrap();
    assert_eq!(v1, 1, "fresh todo starts at version 1");
    let run_id = format!("run-{uniq}");
    p.create_todo_run(&ProjectTodoRunRecord {
        input_snapshot: None,
        trace_manifest: None,
        id: run_id.clone(),
        todo_id: todo.clone(),
        kind: ProjectTodoRunKind::Plan,
        version: v1,
        plan_md: Some("plan".into()),
        output_md: None,
        agent: "plan".into(),
        executor_kind: ProjectExecutorKind::Agent,
        capability_id: None,
        plan_id: None,
        output_ref: None,
        session_id: Some(format!("sess-{uniq}")),
        status: ProjectTodoRunStatus::Running,
        started_at: ts + 5,
        finished_at: None,
        created_at: ts + 5,
    })
    .await
    .unwrap();
    eventually("run v1 bumps next version", || async {
        p.next_todo_version(&todo).await.unwrap() == 2
    })
    .await;
    let run2 = format!("run2-{uniq}");
    p.create_todo_run(&ProjectTodoRunRecord {
        input_snapshot: None,
        trace_manifest: None,
        id: run2.clone(),
        todo_id: todo.clone(),
        kind: ProjectTodoRunKind::Execute,
        version: 2,
        plan_md: None,
        output_md: Some("out".into()),
        agent: "act".into(),
        executor_kind: ProjectExecutorKind::Agent,
        capability_id: None,
        plan_id: None,
        output_ref: None,
        session_id: None,
        status: ProjectTodoRunStatus::Running,
        started_at: ts + 6,
        finished_at: None,
        created_at: ts + 6,
    })
    .await
    .unwrap();
    eventually("runs newest-first", || async {
        let runs = p.list_todo_runs(&todo).await.unwrap();
        let ids: Vec<&str> = runs.iter().map(|r| r.id.as_str()).collect();
        ids == vec![run2.as_str(), run_id.as_str()]
    })
    .await;
    assert!(p
        .patch_todo_run(
            &run_id,
            &opencoder_store::ProjectTodoRunPatch {
                status: Some(ProjectTodoRunStatus::Done),
                finished_at: Some(ts + 7),
                ..Default::default()
            },
            ts + 7,
        )
        .await
        .unwrap());
    eventually("run patch visible", || async {
        p.get_todo_run(&run_id)
            .await
            .unwrap()
            .map(|r| r.status == ProjectTodoRunStatus::Done && r.finished_at == Some(ts + 7))
            .unwrap_or(false)
    })
    .await;
    assert!(
        !p.patch_todo_when(
            &todo,
            ProjectTodoStatus::Planned,
            &ProjectTodoPatch {
                status: Some(ProjectTodoStatus::Failed),
                ..Default::default()
            },
            ts + 7,
        )
        .await
        .unwrap(),
        "todo CAS with a wrong expected status loses"
    );
    assert!(p.claim_todo_running(&todo, ts + 8).await.unwrap());
    assert!(
        p.patch_todo_when(
            &todo,
            ProjectTodoStatus::Running,
            &ProjectTodoPatch {
                status: Some(ProjectTodoStatus::Planned),
                ..Default::default()
            },
            ts + 9,
        )
        .await
        .unwrap(),
        "claim-rollback CAS wins"
    );
    eventually("todo rolled back to planned", || async {
        p.get_todo(&todo)
            .await
            .unwrap()
            .map(|t| t.status == ProjectTodoStatus::Planned)
            .unwrap_or(false)
    })
    .await;
    let atomic_run = ProjectTodoRunRecord {
        input_snapshot: None,
        trace_manifest: None,
        id: format!("atomic-{uniq}"),
        todo_id: todo.clone(),
        kind: ProjectTodoRunKind::Execute,
        version: 3,
        plan_md: Some("atomic plan".into()),
        output_md: None,
        agent: "act".into(),
        executor_kind: ProjectExecutorKind::Dag,
        capability_id: Some(format!("cap-{uniq}")),
        plan_id: Some(format!("plan-{uniq}")),
        output_ref: Some(format!("/workflow/{uniq}/step-1/")),
        session_id: None,
        status: ProjectTodoRunStatus::Running,
        started_at: ts + 10,
        finished_at: None,
        created_at: ts + 10,
    };
    if expect_name == "mysql" {
        assert!(
            !p.claim_todo_running_with_run(&atomic_run, ts + 9)
                .await
                .unwrap(),
            "the existing running attempt prevents another admission"
        );
        assert!(p.get_todo_run(&atomic_run.id).await.unwrap().is_none());
    }
    assert!(
        !p.patch_todo_run_when(
            &run_id,
            ProjectTodoRunStatus::Running,
            &opencoder_store::ProjectTodoRunPatch {
                status: Some(ProjectTodoRunStatus::Failed),
                finished_at: Some(ts + 9),
                ..Default::default()
            },
            ts + 9,
        )
        .await
        .unwrap(),
        "convergence of a terminal run loses"
    );
    assert!(
        p.patch_todo_run_when(
            &run2,
            ProjectTodoRunStatus::Running,
            &opencoder_store::ProjectTodoRunPatch {
                status: Some(ProjectTodoRunStatus::Failed),
                output_md: Some("converged".into()),
                finished_at: Some(ts + 9),
                ..Default::default()
            },
            ts + 9,
        )
        .await
        .unwrap(),
        "convergence of a running run wins"
    );
    if expect_name == "mysql" {
        assert!(p
            .claim_todo_running_with_run(&atomic_run, ts + 10)
            .await
            .unwrap());
        let claimed = p.get_todo_run(&atomic_run.id).await.unwrap().unwrap();
        assert_eq!(claimed.executor_kind, ProjectExecutorKind::Dag);
        assert_eq!(claimed.capability_id, Some(format!("cap-{uniq}")));
        assert_eq!(claimed.plan_id, Some(format!("plan-{uniq}")));
        assert_eq!(
            claimed.output_ref,
            Some(format!("/workflow/{uniq}/step-1/"))
        );
        assert_eq!(
            p.get_todo(&todo).await.unwrap().unwrap().status,
            ProjectTodoStatus::Running
        );
        assert!(p
            .patch_todo_when(
                &todo,
                ProjectTodoStatus::Running,
                &ProjectTodoPatch {
                    status: Some(ProjectTodoStatus::Planned),
                    ..Default::default()
                },
                ts + 11,
            )
            .await
            .unwrap());
    } else {
        let error = p
            .claim_todo_running_with_run(&atomic_run, ts + 10)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("starrocks does not support atomic"));
        assert!(p.get_todo_run(&atomic_run.id).await.unwrap().is_none());
        assert_eq!(
            p.get_todo(&todo).await.unwrap().unwrap().status,
            ProjectTodoStatus::Planned
        );
    }
    assert!(p
        .patch_todo(
            &todo,
            &ProjectTodoPatch {
                plan_md: Some(Some("a plan".into())),
                ..Default::default()
            },
            ts + 7,
        )
        .await
        .unwrap());
    eventually("plan_md set visible", || async {
        p.get_todo(&todo)
            .await
            .unwrap()
            .map(|t| t.plan_md.as_deref() == Some("a plan"))
            .unwrap_or(false)
    })
    .await;
    assert!(p
        .patch_todo(
            &todo,
            &ProjectTodoPatch {
                plan_md: Some(None),
                ..Default::default()
            },
            ts + 8,
        )
        .await
        .unwrap());
    eventually("plan_md cleared to NULL", || async {
        p.get_todo(&todo)
            .await
            .unwrap()
            .map(|t| t.plan_md.is_none())
            .unwrap_or(false)
    })
    .await;
    let step_run = ProjectTodoRunRecord {
        input_snapshot: None,
        trace_manifest: None,
        id: format!("step-{uniq}"),
        todo_id: todo.clone(),
        kind: ProjectTodoRunKind::Step,
        version: 9,
        plan_md: None,
        output_md: None,
        agent: "act".into(),
        executor_kind: ProjectExecutorKind::Agent,
        capability_id: None,
        plan_id: None,
        output_ref: None,
        session_id: None,
        status: ProjectTodoRunStatus::Running,
        started_at: ts + 12,
        finished_at: None,
        created_at: ts + 12,
    };
    p.create_todo_run(&step_run).await.unwrap();
    assert!(p
        .finish_todo_run(
            &step_run.id,
            &opencoder_store::ProjectTodoRunPatch {
                status: Some(ProjectTodoRunStatus::Done),
                output_md: Some("step out".into()),
                finished_at: Some(ts + 13),
                ..Default::default()
            },
            ts + 13,
        )
        .await
        .unwrap());
    eventually("step run converged, todo untouched", || async {
        p.get_todo_run(&step_run.id)
            .await
            .unwrap()
            .map(|r| r.status == ProjectTodoRunStatus::Done)
            .unwrap_or(false)
            && p.get_todo(&todo)
                .await
                .unwrap()
                .map(|t| t.status == ProjectTodoStatus::Planned)
                .unwrap_or(false)
    })
    .await;
    assert!(p.delete_todo(&todo).await.unwrap());
    eventually("todo + runs gone", || async {
        p.get_todo(&todo).await.unwrap().is_none()
            && p.list_todo_runs(&todo).await.unwrap().is_empty()
    })
    .await;
    assert!(!p.delete_todo(&todo).await.unwrap(), "second delete false");
    assert!(p.delete_goal(&goal).await.unwrap());
    eventually("goal + milestones gone", || async {
        p.list_initiatives(Some(&goal)).await.unwrap().is_empty()
            && !p.list_goals().await.unwrap().iter().any(|g| g.id == goal)
    })
    .await;
    let retained = p.list_initiatives(None).await.unwrap();
    assert!(retained.iter().any(|m| m.id == ms && m.goal_id.is_none()));
    assert!(p.delete_initiative(&ms).await.unwrap());
    assert!(!p.delete_goal(&goal).await.unwrap(), "second delete false");
    assert!(p.delete_goal(&goal2).await.unwrap(), "cleanup second goal");
}

#[path = "gated/suite_1.rs"]
mod suite_1;

#[path = "gated/goals.rs"]
mod goals;

#[path = "gated/helpers.rs"]
mod helpers;
use helpers::{env_dsn, eventually, storage};
