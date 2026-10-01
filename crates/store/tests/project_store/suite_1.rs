use super::*;

#[tokio::test]
async fn goal_crud_patch_and_missing_id() {
    let (_dir, _store, p) = fresh().await;
    p.create_goal(&goal("g1", 2, 100)).await.unwrap();
    p.create_goal(&goal("g2", 1, 200)).await.unwrap();

    // Ordered by sort first, created_at as tiebreak.
    let goals = p.list_goals().await.unwrap();
    assert_eq!(goals.len(), 2);
    assert_eq!(goals[0].id, "g2");
    assert_eq!(goals[1].id, "g1");

    let ok = p
        .patch_goal(
            "g1",
            &ProjectGoalPatch {
                title: Some("renamed".to_string()),
                detail_md: Some("details".to_string()),
                status: Some(ProjectGoalStatus::Archived),
                sort: Some(9),
            },
            999,
        )
        .await
        .unwrap();
    assert!(ok);
    let g1 = &p.list_goals().await.unwrap()[1];
    assert_eq!(g1.title, "renamed");
    assert_eq!(g1.detail_md.as_deref(), Some("details"));
    assert_eq!(g1.status, ProjectGoalStatus::Archived);
    assert_eq!(g1.sort, 9);
    assert_eq!(g1.updated_at, 999);
    assert!(g1.status.is_terminal());

    // Missing ids: false, not an error.
    assert!(!p
        .patch_goal("nope", &ProjectGoalPatch::default(), 1)
        .await
        .unwrap());
    assert!(!p.delete_goal("nope").await.unwrap());
}

#[tokio::test]
async fn initiative_crud_and_goal_filter() {
    let (_dir, _store, p) = fresh().await;
    p.create_goal(&goal("g1", 0, 1)).await.unwrap();
    p.create_goal(&goal("g2", 0, 2)).await.unwrap();
    p.create_initiative(&initiative("m1", "g1", 2, 10))
        .await
        .unwrap();
    p.create_initiative(&initiative("m2", "g1", 1, 20))
        .await
        .unwrap();
    p.create_initiative(&initiative("m3", "g2", 0, 30))
        .await
        .unwrap();

    let for_g1 = p.list_initiatives(Some("g1")).await.unwrap();
    assert_eq!(
        for_g1.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["m2", "m1"],
        "filtered by goal and ordered by sort_key"
    );
    assert_eq!(p.list_initiatives(None).await.unwrap().len(), 3);

    assert!(p
        .patch_initiative(
            "m1",
            &ProjectInitiativePatch {
                title: Some("renamed".to_string()),
                status: Some(ProjectInitiativeStatus::InProgress),
                ..Default::default()
            },
            77,
        )
        .await
        .unwrap());
    let m1 = p.list_initiatives(Some("g1")).await.unwrap()[1].clone();
    assert_eq!(m1.title, "renamed");
    assert_eq!(m1.status, ProjectInitiativeStatus::InProgress);
    assert_eq!(m1.updated_at, 77);
    assert!(!m1.status.is_terminal());
}

#[tokio::test]
async fn todo_patch_semantics_including_clear_to_null() {
    let (_dir, _store, p) = fresh().await;
    p.create_goal(&goal("g1", 0, 1)).await.unwrap();
    p.create_initiative(&initiative("m1", "g1", 0, 2))
        .await
        .unwrap();
    p.create_todo(&todo("t1", Some("m1"), 3)).await.unwrap();
    p.create_todo(&todo("t2", None, 4)).await.unwrap();

    // Backlog + initiative todos are both covered by the unfiltered list.
    assert_eq!(p.list_todos(None).await.unwrap().len(), 2);
    let for_m1 = p.list_todos(Some("m1")).await.unwrap();
    assert_eq!(for_m1.len(), 1);
    assert_eq!(for_m1[0].id, "t1");

    // Set plan_md + active_session_id, then clear them to NULL.
    assert!(p
        .patch_todo(
            "t1",
            &ProjectTodoPatch {
                plan_md: Some(Some("plan v1".to_string())),
                active_session_id: Some(Some("sess-1".to_string())),
                status: Some(ProjectTodoStatus::Running),
                ..Default::default()
            },
            50,
        )
        .await
        .unwrap());
    let t1 = p.get_todo("t1").await.unwrap().unwrap();
    assert_eq!(t1.plan_md.as_deref(), Some("plan v1"));
    assert_eq!(t1.active_session_id.as_deref(), Some("sess-1"));
    assert_eq!(t1.status, ProjectTodoStatus::Running);

    assert!(p
        .patch_todo(
            "t1",
            &ProjectTodoPatch {
                plan_md: Some(None),
                active_session_id: Some(None),
                initiative_id: Some(None), // back to the backlog
                ..Default::default()
            },
            60,
        )
        .await
        .unwrap());
    let t1 = p.get_todo("t1").await.unwrap().unwrap();
    assert_eq!(t1.plan_md, None, "Some(None) clears plan_md to NULL");
    assert_eq!(t1.active_session_id, None);
    assert_eq!(t1.initiative_id, None, "Some(None) clears initiative_id");
    assert_eq!(t1.updated_at, 60);

    assert!(p.get_todo("missing").await.unwrap().is_none());
    assert!(!p
        .patch_todo("missing", &ProjectTodoPatch::default(), 1)
        .await
        .unwrap());
}

#[tokio::test]
async fn run_versions_and_listing_order() {
    let (_dir, _store, p) = fresh().await;
    p.create_todo(&todo("t1", None, 1)).await.unwrap();

    // Empty todo starts at version 1.
    assert_eq!(p.next_todo_version("t1").await.unwrap(), 1);
    run(p.as_ref(), "r1", "t1", 10).await;
    assert_eq!(p.next_todo_version("t1").await.unwrap(), 2);
    run(p.as_ref(), "r2", "t1", 20).await;
    assert_eq!(p.next_todo_version("t1").await.unwrap(), 3);

    // Newest version first.
    let runs = p.list_todo_runs("t1").await.unwrap();
    assert_eq!(
        runs.iter().map(|r| r.version).collect::<Vec<_>>(),
        vec![2, 1]
    );
    assert_eq!(runs[0].id, "r2");

    assert!(p
        .patch_todo_run(
            "r2",
            &ProjectTodoRunPatch {
                status: Some(ProjectTodoRunStatus::Done),
                finished_at: Some(99),
                output_md: Some("done".to_string()),
                ..Default::default()
            },
            99,
        )
        .await
        .unwrap());
    let r2 = p.get_todo_run("r2").await.unwrap().unwrap();
    assert_eq!(r2.status, ProjectTodoRunStatus::Done);
    assert_eq!(r2.finished_at, Some(99));
    assert!(r2.status.is_terminal());
    assert!(p.get_todo_run("missing").await.unwrap().is_none());
}

#[tokio::test]
async fn claim_todo_running_cas_and_running_run_listing() {
    let (_dir, _store, p) = fresh().await;
    p.create_todo(&todo("t1", None, 1)).await.unwrap();
    assert!(p
        .patch_todo(
            "t1",
            &ProjectTodoPatch {
                status: Some(ProjectTodoStatus::Planned),
                ..Default::default()
            },
            10,
        )
        .await
        .unwrap());

    // Planned -> the claim wins and flips the row to running.
    assert!(p.claim_todo_running("t1", 20).await.unwrap());
    let t1 = p.get_todo("t1").await.unwrap().unwrap();
    assert_eq!(t1.status, ProjectTodoStatus::Running);
    assert_eq!(t1.updated_at, 20);

    // Already running -> no claim; the row is not re-stamped.
    assert!(!p.claim_todo_running("t1", 30).await.unwrap());
    let t1 = p.get_todo("t1").await.unwrap().unwrap();
    assert_eq!(t1.status, ProjectTodoStatus::Running);
    assert_eq!(t1.updated_at, 20, "a lost claim must not re-stamp");

    // Unknown id -> no claim, not an error (patch_* convention).
    assert!(!p.claim_todo_running("nope", 40).await.unwrap());

    // list_running_todo_runs: only the running rows, across todos.
    p.create_todo(&todo("t2", None, 2)).await.unwrap();
    run(p.as_ref(), "r1", "t1", 50).await; // helper seeds status running
    run(p.as_ref(), "r2", "t2", 60).await;
    assert!(p
        .patch_todo_run(
            "r2",
            &ProjectTodoRunPatch {
                status: Some(ProjectTodoRunStatus::Done),
                finished_at: Some(99),
                ..Default::default()
            },
            99,
        )
        .await
        .unwrap());
    let running = p.list_running_todo_runs().await.unwrap();
    assert_eq!(
        running.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["r1"],
        "only the still-running run row is listed"
    );
    assert_eq!(running[0].todo_id, "t1");
    assert_eq!(running[0].status, ProjectTodoRunStatus::Running);
}
