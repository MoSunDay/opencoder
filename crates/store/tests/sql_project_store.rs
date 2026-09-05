//! Integration test for the feature-gated MySQL / StarRocks project store.
//!
//! The whole body lives in a `cfg`-gated module so default-feature builds
//! compile this file empty (no sqlx). The live tests read their DSN from the
//! environment (`OC_TEST_MYSQL_DSN` / `OC_TEST_STARROCKS_DSN`) and SKIP with
//! a warning when unset — no credentials ever live in the repo. When set,
//! they run one compact CRUD contract through the `Arc<dyn ProjectStore>`
//! seam: create goal → patch → milestone → todo → run v1 via
//! next_todo_version → list orders → plan_md clear-to-NULL → expected-status
//! CAS (todo claim-rollback + terminal-run convergence) → cascade deletes.

#[cfg(any(feature = "mysql", feature = "starrocks"))]
mod gated {
    use std::future::Future;
    use std::time::Duration;

    use opencoder_core::{StorageBackend, StorageConfig};
    use opencoder_store::sql_store;
    use opencoder_store::{
        ProjectGoalPatch, ProjectGoalRecord, ProjectGoalStatus, ProjectMilestoneRecord,
        ProjectMilestoneStatus, ProjectStore, ProjectTodoPatch, ProjectTodoRecord,
        ProjectTodoRunKind, ProjectTodoRunRecord, ProjectTodoRunStatus, ProjectTodoStatus,
    };

    /// Non-empty value of `var`, if set.
    fn env_dsn(var: &str) -> Option<String> {
        std::env::var(var)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }

    /// Poll `probe` until it reports success. StarRocks publishes committed
    /// writes asynchronously, so a read immediately following a write can
    /// briefly observe the pre-write state; MySQL satisfies every probe on
    /// the first attempt.
    async fn eventually<F, Fut>(what: &str, mut probe: F)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = bool>,
    {
        for _ in 0..100 {
            if probe().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("timed out waiting for {what}");
    }

    /// The CRUD contract, shared by both backend variants.
    async fn crud_contract(p: &dyn ProjectStore, uniq: &str, expect_name: &str) {
        assert_eq!(p.project_backend_name(), expect_name);
        let (goal, goal2, ms, todo) = (
            format!("goal-{uniq}"),
            format!("goal2-{uniq}"),
            format!("ms-{uniq}"),
            format!("todo-{uniq}"),
        );
        let ts = 1_000i64;

        // goal create + patch; list orders by sort then created_at.
        p.create_goal(&ProjectGoalRecord {
            id: goal.clone(),
            title: "goal".into(),
            detail_md: None,
            status: ProjectGoalStatus::Active,
            sort: 2,
            created_at: ts,
            updated_at: ts,
        })
        .await
        .unwrap();
        p.create_goal(&ProjectGoalRecord {
            id: goal2.clone(),
            title: "goal two".into(),
            detail_md: None,
            status: ProjectGoalStatus::Active,
            sort: 1,
            created_at: ts + 1,
            updated_at: ts + 1,
        })
        .await
        .unwrap();
        let ok = p
            .patch_goal(
                &goal,
                &ProjectGoalPatch {
                    title: Some("renamed".into()),
                    detail_md: Some("details".into()),
                    status: Some(ProjectGoalStatus::Active),
                    sort: Some(2),
                },
                ts + 2,
            )
            .await
            .unwrap();
        assert!(ok, "patch of a live goal reports true");
        assert!(
            !p.patch_goal(&format!("missing-{uniq}"), &ProjectGoalPatch::default(), ts)
                .await
                .unwrap(),
            "patch of a missing goal reports false"
        );
        let goals_ok = || async {
            let goals = p.list_goals().await.unwrap();
            let Some(mine) = goals.iter().find(|g| g.id == goal) else {
                return false;
            };
            mine.title == "renamed"
                && mine.detail_md.as_deref() == Some("details")
                && mine.updated_at == ts + 2
                && match (
                    goals.iter().position(|g| g.id == goal2),
                    goals.iter().position(|g| g.id == goal),
                ) {
                    (Some(p2), Some(p1)) => p2 < p1,
                    _ => false,
                }
        };
        eventually("goal patch visible + list order", goals_ok).await;

        // milestone under the goal.
        p.create_milestone(&ProjectMilestoneRecord {
            id: ms.clone(),
            goal_id: goal.clone(),
            title: "ms".into(),
            detail_md: None,
            status: ProjectMilestoneStatus::Planned,
            sort: 1,
            created_at: ts + 3,
            updated_at: ts + 3,
        })
        .await
        .unwrap();
        let ms_ok = || async {
            let mss = p.list_milestones(Some(&goal)).await.unwrap();
            mss.len() == 1 && mss[0].id == ms
        };
        eventually("milestone visible under goal", ms_ok).await;

        // todo under the milestone; run v1 allocated via next_todo_version.
        p.create_todo(&ProjectTodoRecord {
            id: todo.clone(),
            milestone_id: Some(ms.clone()),
            title: "todo".into(),
            draft: "draft".into(),
            plan_md: None,
            status: ProjectTodoStatus::Draft,
            agent: "act".into(),
            active_session_id: None,
            created_at: ts + 4,
            updated_at: ts + 4,
        })
        .await
        .unwrap();
        let todo_ok = || async {
            let todos = p.list_todos(Some(&ms)).await.unwrap();
            todos.len() == 1 && todos[0].id == todo
        };
        eventually("todo visible under milestone", todo_ok).await;

        let v1 = p.next_todo_version(&todo).await.unwrap();
        assert_eq!(v1, 1, "fresh todo starts at version 1");
        let run_id = format!("run-{uniq}");
        p.create_todo_run(&ProjectTodoRunRecord {
            id: run_id.clone(),
            todo_id: todo.clone(),
            kind: ProjectTodoRunKind::Plan,
            version: v1,
            plan_md: Some("plan".into()),
            output_md: None,
            agent: "plan".into(),
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
            id: run2.clone(),
            todo_id: todo.clone(),
            kind: ProjectTodoRunKind::Execute,
            version: 2,
            plan_md: None,
            output_md: Some("out".into()),
            agent: "act".into(),
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

        // Expected-status CAS pair (todo + run) through the same seam. This
        // contract only runs against a live server (DSN-gated above — it
        // env-skips locally because no MySQL/StarRocks is running there);
        // writes assert their boolean result directly, read-backs that must
        // observe a write go through `eventually` (StarRocks commits async).
        // Lost CAS: the todo is still Draft, so expecting Planned loses and
        // must not flip it to Failed.
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
        // Won CAS: claim -> Running, then a CAS expecting Running rolls the
        // row back to Planned (the claim-rollback shape).
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
        // Run CAS: converging the terminal run (Done above) while expecting
        // Running loses; the still-Running run2 converges to Failed.
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

        // plan_md set, then cleared to NULL via Some(None).
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

        // delete_todo cascades its runs.
        assert!(p.delete_todo(&todo).await.unwrap());
        eventually("todo + runs gone", || async {
            p.get_todo(&todo).await.unwrap().is_none()
                && p.list_todo_runs(&todo).await.unwrap().is_empty()
        })
        .await;
        assert!(!p.delete_todo(&todo).await.unwrap(), "second delete false");

        // delete_goal cascades milestones (and any todos left under them).
        assert!(p.delete_goal(&goal).await.unwrap());
        eventually("goal + milestones gone", || async {
            p.list_milestones(Some(&goal)).await.unwrap().is_empty()
                && !p.list_goals().await.unwrap().iter().any(|g| g.id == goal)
        })
        .await;
        assert!(!p.delete_goal(&goal).await.unwrap(), "second delete false");
        assert!(p.delete_goal(&goal2).await.unwrap(), "cleanup second goal");
    }

    fn storage(backend: StorageBackend, dsn: &str) -> StorageConfig {
        StorageConfig {
            backend,
            mysql: Some(dsn.to_string()).filter(|_| backend == StorageBackend::Mysql),
            starrocks: Some(dsn.to_string()).filter(|_| backend == StorageBackend::Starrocks),
        }
    }

    #[tokio::test]
    async fn mysql_project_crud_contract() {
        let Some(dsn) = env_dsn("OC_TEST_MYSQL_DSN") else {
            eprintln!("warning: OC_TEST_MYSQL_DSN not set — skipping sql_store integration test");
            return;
        };
        let p = sql_store::open(&storage(StorageBackend::Mysql, &dsn))
            .await
            .expect("open mysql project store");
        crud_contract(p.as_ref(), &ulid::Ulid::new().to_string(), "mysql").await;
    }

    #[cfg(feature = "starrocks")]
    #[tokio::test]
    async fn starrocks_project_crud_contract() {
        let Some(dsn) = env_dsn("OC_TEST_STARROCKS_DSN") else {
            eprintln!(
                "warning: OC_TEST_STARROCKS_DSN not set — skipping sql_store integration test"
            );
            return;
        };
        let p = sql_store::open(&storage(StorageBackend::Starrocks, &dsn))
            .await
            .expect("open starrocks project store");
        crud_contract(p.as_ref(), &ulid::Ulid::new().to_string(), "starrocks").await;
    }
}
