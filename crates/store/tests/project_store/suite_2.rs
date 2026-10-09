use super::*;

#[tokio::test]
async fn conditional_patch_cas_applies_only_in_expected_state() {
    let (_dir, _store, p) = fresh().await;
    p.create_todo(&todo("t1", None, 1)).await.unwrap();

    // Lost CAS (wrong expected status): false, and the row is untouched —
    // still Draft with its original updated_at.
    assert!(!p
        .patch_todo_when(
            "t1",
            ProjectTodoStatus::Planned,
            &ProjectTodoPatch {
                status: Some(ProjectTodoStatus::Failed),
                ..Default::default()
            },
            10,
        )
        .await
        .unwrap());
    let t1 = p.get_todo("t1").await.unwrap().unwrap();
    assert_eq!(t1.status, ProjectTodoStatus::Draft);
    assert_eq!(t1.updated_at, 1, "a lost CAS must not re-stamp");

    // Won CAS: matching expected status applies and flips the row.
    assert!(p
        .patch_todo_when(
            "t1",
            ProjectTodoStatus::Draft,
            &ProjectTodoPatch {
                status: Some(ProjectTodoStatus::Planned),
                ..Default::default()
            },
            11,
        )
        .await
        .unwrap());
    assert_eq!(
        p.get_todo("t1").await.unwrap().unwrap().status,
        ProjectTodoStatus::Planned
    );

    // Claim-rollback shape: the claim wins (Running), then a CAS still
    // expecting Running rolls the row back to the pre-claim status.
    assert!(p.claim_todo_running("t1", 12).await.unwrap());
    assert!(p
        .patch_todo_when(
            "t1",
            ProjectTodoStatus::Running,
            &ProjectTodoPatch {
                status: Some(ProjectTodoStatus::Planned),
                ..Default::default()
            },
            13,
        )
        .await
        .unwrap());

    // Unknown id: false, not an error (patch_* convention).
    assert!(!p
        .patch_todo_when(
            "missing",
            ProjectTodoStatus::Running,
            &ProjectTodoPatch {
                status: Some(ProjectTodoStatus::Failed),
                ..Default::default()
            },
            14,
        )
        .await
        .unwrap());

    // Plan-writeback racing execute: after a fresh claim the todo is
    // Running, so a writeback still expecting Planned loses and must not
    // clobber plan_md.
    assert!(p.claim_todo_running("t1", 15).await.unwrap());
    assert!(!p
        .patch_todo_when(
            "t1",
            ProjectTodoStatus::Planned,
            &ProjectTodoPatch {
                status: Some(ProjectTodoStatus::Planned),
                plan_md: Some(Some("# new".to_string())),
                ..Default::default()
            },
            15,
        )
        .await
        .unwrap());
    let t1 = p.get_todo("t1").await.unwrap().unwrap();
    assert_eq!(t1.status, ProjectTodoStatus::Running);
    assert_eq!(t1.plan_md, None, "a lost plan writeback keeps plan_md");

    // Run CAS: seed a running run, then try to converge it as if it were
    // already Done — lost, the row stays Running/finished_at None.
    run(p.as_ref(), "r1", "t1", 20).await;
    assert!(!p
        .patch_todo_run_when(
            "r1",
            ProjectTodoRunStatus::Done,
            &ProjectTodoRunPatch {
                status: Some(ProjectTodoRunStatus::Failed),
                finished_at: Some(99),
                ..Default::default()
            },
            20,
        )
        .await
        .unwrap());
    let r1 = p.get_todo_run("r1").await.unwrap().unwrap();
    assert_eq!(r1.status, ProjectTodoRunStatus::Running);
    assert_eq!(r1.finished_at, None);

    // Panic/stale convergence on a running row: wins, flips to Failed.
    assert!(p
        .patch_todo_run_when(
            "r1",
            ProjectTodoRunStatus::Running,
            &ProjectTodoRunPatch {
                status: Some(ProjectTodoRunStatus::Failed),
                output_md: Some("converged".to_string()),
                finished_at: Some(21),
                ..Default::default()
            },
            21,
        )
        .await
        .unwrap());
    let r1 = p.get_todo_run("r1").await.unwrap().unwrap();
    assert_eq!(r1.status, ProjectTodoRunStatus::Failed);
    assert_eq!(r1.output_md.as_deref(), Some("converged"));
    assert_eq!(r1.finished_at, Some(21));

    // The same convergence replayed on the now-terminal row must not
    // relabel it — the row is no longer Running.
    assert!(!p
        .patch_todo_run_when(
            "r1",
            ProjectTodoRunStatus::Running,
            &ProjectTodoRunPatch {
                status: Some(ProjectTodoRunStatus::Failed),
                output_md: Some("converged".to_string()),
                finished_at: Some(21),
                ..Default::default()
            },
            22,
        )
        .await
        .unwrap());
    assert_eq!(
        p.get_todo_run("r1").await.unwrap().unwrap().status,
        ProjectTodoRunStatus::Failed
    );
}

#[tokio::test]
async fn delete_goal_preserves_initiative_todo_and_runs() {
    let (_dir, _store, p) = fresh().await;
    p.create_goal(&goal("g1", 0, 1)).await.unwrap();
    p.create_initiative(&initiative("m1", "g1", 0, 2))
        .await
        .unwrap();
    p.create_todo(&todo("t1", Some("m1"), 3)).await.unwrap();
    run(p.as_ref(), "r1", "t1", 4).await;

    assert!(p.delete_goal("g1").await.unwrap());
    assert!(p.list_goals().await.unwrap().is_empty());
    let initiatives = p.list_initiatives(None).await.unwrap();
    assert_eq!(initiatives.len(), 1);
    assert_eq!(initiatives[0].goal_id, None);
    assert_eq!(p.list_todos(None).await.unwrap().len(), 1);
    assert_eq!(p.list_todo_runs("t1").await.unwrap().len(), 1);
    assert_eq!(
        p.get_todo("t1")
            .await
            .unwrap()
            .unwrap()
            .initiative_id
            .as_deref(),
        Some("m1")
    );
}

#[tokio::test]
async fn delete_todo_cascades_runs() {
    let (_dir, _store, p) = fresh().await;
    p.create_todo(&todo("t1", None, 1)).await.unwrap();
    run(p.as_ref(), "r1", "t1", 2).await;
    run(p.as_ref(), "r2", "t1", 3).await;

    assert!(p.delete_todo("t1").await.unwrap());
    assert!(p.list_todos(None).await.unwrap().is_empty());
    assert!(p.list_todo_runs("t1").await.unwrap().is_empty());
    assert!(!p.delete_todo("t1").await.unwrap(), "second delete: gone");
}

#[tokio::test]
async fn delete_initiative_requires_explicit_unlink_and_preserves_runs() {
    let (_dir, _store, p) = fresh().await;
    p.create_goal(&goal("g1", 0, 1)).await.unwrap();
    p.create_initiative(&initiative("m1", "g1", 0, 2))
        .await
        .unwrap();
    p.create_initiative(&initiative("m2", "g1", 1, 3))
        .await
        .unwrap();
    p.create_todo(&todo("t1", Some("m1"), 4)).await.unwrap();
    p.create_todo(&todo("t2", Some("m2"), 5)).await.unwrap();
    run(p.as_ref(), "r1", "t1", 6).await;

    assert!(p
        .delete_initiative("m1")
        .await
        .unwrap_err()
        .is::<opencoder_store::project::InitiativeNotEmpty>());
    assert_eq!(p.list_initiatives(None).await.unwrap().len(), 2);
    assert_eq!(
        p.get_todo("t1")
            .await
            .unwrap()
            .unwrap()
            .initiative_id
            .as_deref(),
        Some("m1")
    );
    assert_eq!(p.list_todo_runs("t1").await.unwrap().len(), 1);
    p.patch_todo(
        "t1",
        &ProjectTodoPatch {
            initiative_id: Some(None),
            ..Default::default()
        },
        7,
    )
    .await
    .unwrap();
    assert!(p.delete_initiative("m1").await.unwrap());
    assert_eq!(p.list_todos(None).await.unwrap().len(), 2);
    assert_eq!(p.list_todo_runs("t1").await.unwrap().len(), 1);
    assert_eq!(p.list_initiatives(Some("g1")).await.unwrap().len(), 1);
}

#[tokio::test]
async fn reopen_is_idempotent_and_serves_v15() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate.db");

    // First open creates the file at the latest schema.
    {
        let store = LibsqlStore::open(&db_path).await.unwrap();
        drop(store);
    }
    // Second open re-runs bootstrap/migrate on the existing file; creating and
    // listing a goal proves the v15 tables are live after the reopen.
    let store = LibsqlStore::open(&db_path).await.unwrap();
    let conn = store.conn().await.unwrap();
    let stmt = conn
        .prepare("SELECT version FROM schema_version LIMIT 1")
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let v: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
    assert_eq!(v, 34, "schema_version must be latest (34) after reopen");

    let iface: Arc<dyn ProjectStore> = Arc::new(store);
    iface.create_goal(&goal("g1", 0, 1)).await.unwrap();
    let goals = iface.list_goals().await.unwrap();
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].id, "g1");

    // Third open: still idempotent, data intact.
    drop(iface);
    let store3 = LibsqlStore::open(&db_path).await.unwrap();
    let iface3: Arc<dyn ProjectStore> = Arc::new(store3);
    assert_eq!(iface3.list_goals().await.unwrap().len(), 1);
}
