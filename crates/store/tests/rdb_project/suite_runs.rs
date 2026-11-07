//! Run versioning, listing order, CAS patches and atomic todo claims.
use super::*;

#[tokio::test]
async fn run_versions_and_listing_order() {
    let p = mem_iface().await;
    p.create_todo(&todo("t1", None, 1)).await.unwrap();

    // Empty todo starts at version 1; each run bumps it.
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
                output_md: Some("done".to_string()),
                ..rpatch(Some(ProjectTodoRunStatus::Done), Some(99))
            },
            99,
        )
        .await
        .unwrap());
    let r2 = p.get_todo_run("r2").await.unwrap().unwrap();
    assert_eq!(r2.status, ProjectTodoRunStatus::Done);
    assert_eq!(r2.finished_at, Some(99));
    assert_eq!(r2.output_md.as_deref(), Some("done"));
    assert!(!p
        .patch_todo_run("missing", &rpatch(None, None), 1)
        .await
        .unwrap());
    // Duplicate run id is rejected before commit.
    assert!(p
        .create_todo_run(&run_record("r1", "t1", 9, 1, ProjectTodoRunKind::Plan))
        .await
        .is_err());
}

#[tokio::test]
async fn claim_todo_running_cas_and_running_run_listing() {
    let p = mem_iface().await;
    p.create_todo(&todo("t1", None, 1)).await.unwrap();
    p.patch_todo("t1", &tpatch(Some(ProjectTodoStatus::Planned)), 10)
        .await
        .unwrap();

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

    // list_running_todo_runs: only still-running rows, across todos.
    p.create_todo(&todo("t2", None, 2)).await.unwrap();
    run(p.as_ref(), "r1", "t1", 50).await;
    run(p.as_ref(), "r2", "t2", 60).await;
    assert!(p
        .patch_todo_run(
            "r2",
            &rpatch(Some(ProjectTodoRunStatus::Done), Some(99)),
            99
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

#[tokio::test]
async fn conditional_patch_cas_applies_only_in_expected_state() {
    let p = mem_iface().await;
    p.create_todo(&todo("t1", None, 1)).await.unwrap();

    // Lost CAS (wrong expected status): false, and the row is untouched —
    // still Draft with its original updated_at.
    assert!(!p
        .patch_todo_when(
            "t1",
            ProjectTodoStatus::Planned,
            &tpatch(Some(ProjectTodoStatus::Failed)),
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
            &tpatch(Some(ProjectTodoStatus::Planned)),
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
            &tpatch(Some(ProjectTodoStatus::Planned)),
            13,
        )
        .await
        .unwrap());

    // Unknown id: false, not an error (patch_* convention).
    assert!(!p
        .patch_todo_when("missing", ProjectTodoStatus::Planned, &tpatch(None), 14)
        .await
        .unwrap());

    // Stale plan writeback: todo already Running, so a CAS expecting
    // Planned loses and plan_md keeps its NULL.
    assert!(p.claim_todo_running("t1", 14).await.unwrap());
    assert!(!p
        .patch_todo_when(
            "t1",
            ProjectTodoStatus::Planned,
            &ProjectTodoPatch {
                plan_md: Some(Some("# new".to_string())),
                ..tpatch(Some(ProjectTodoStatus::Planned))
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
                output_md: Some("late".to_string()),
                ..rpatch(Some(ProjectTodoRunStatus::Failed), Some(99))
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
                output_md: Some("converged".to_string()),
                ..rpatch(Some(ProjectTodoRunStatus::Failed), Some(21))
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
            &rpatch(Some(ProjectTodoRunStatus::Done), None),
            22,
        )
        .await
        .unwrap());
    assert_eq!(
        p.get_todo_run("r1").await.unwrap().unwrap().status,
        ProjectTodoRunStatus::Failed
    );
}

/// Shared seeder for the finish tests: a Running todo plus a fresh run of
/// `kind` at the next version.
pub(super) async fn seed_running_run(
    p: &dyn ProjectStore,
    todo_id: &str,
    run_id: &str,
    created_at: i64,
    kind: ProjectTodoRunKind,
) {
    p.patch_todo(
        todo_id,
        &tpatch(Some(ProjectTodoStatus::Planned)),
        created_at,
    )
    .await
    .unwrap();
    assert!(p.claim_todo_running(todo_id, created_at + 1).await.unwrap());
    let version = p.next_todo_version(todo_id).await.unwrap();
    let mut rec = run_record(run_id, todo_id, version, created_at + 2, kind);
    rec.session_id = Some(format!("sess-{run_id}"));
    p.create_todo_run(&rec).await.unwrap();
}
