//! Functional tests for the project-module store API (goals / milestones /
//! todos / runs) against the libsql backend, exercised through the
//! `Arc<dyn ProjectStore>` seam upper layers will use.
//!
//! Behavior contracts:
//! - goal_crud_patch_and_missing_id: CRUD + patch (title/detail/status/sort),
//!   patch/delete of a missing id returns false
//! - milestone_crud_and_goal_filter: CRUD under a goal + list_milestones filter
//! - todo_patch_semantics_including_clear_to_null: Option<Option<String>>
//!   clears plan_md/milestone_id to NULL; backlog vs milestone todos list
//! - run_versions_and_listing_order: next_todo_version numbering (empty = 1),
//!   newest-first listing, patch stamps status/finished_at
//! - cascades: delete_goal removes milestone+todo+runs; delete_todo removes
//!   runs; delete_milestone removes its todos and their runs (no re-parent)
//! - reopen_is_idempotent_and_serves_v15: second `open` on the same file
//!   migrates 14→15 cleanly and the project tables keep working
//! - libsql_store_coerces_to_project_store: compile-level trait-object check

use std::sync::Arc;

use opencoder_store::{
    LibsqlStore, ProjectGoalPatch, ProjectGoalRecord, ProjectGoalStatus, ProjectMilestonePatch,
    ProjectMilestoneRecord, ProjectMilestoneStatus, ProjectStore, ProjectTodoPatch,
    ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunPatch, ProjectTodoRunRecord,
    ProjectTodoRunStatus, ProjectTodoStatus,
};

async fn fresh() -> (tempfile::TempDir, Arc<LibsqlStore>, Arc<dyn ProjectStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(LibsqlStore::open(dir.path().join("test.db")).await.unwrap());
    let iface: Arc<dyn ProjectStore> = store.clone();
    (dir, store, iface)
}

fn goal(id: &str, sort: i64, created_at: i64) -> ProjectGoalRecord {
    ProjectGoalRecord {
        id: id.to_string(),
        title: format!("goal {id}"),
        detail_md: None,
        status: ProjectGoalStatus::Active,
        sort,
        created_at,
        updated_at: created_at,
    }
}

fn milestone(id: &str, goal_id: &str, sort: i64, created_at: i64) -> ProjectMilestoneRecord {
    ProjectMilestoneRecord {
        id: id.to_string(),
        goal_id: goal_id.to_string(),
        title: format!("milestone {id}"),
        detail_md: None,
        status: ProjectMilestoneStatus::Planned,
        sort,
        created_at,
        updated_at: created_at,
    }
}

fn todo(id: &str, milestone_id: Option<&str>, created_at: i64) -> ProjectTodoRecord {
    ProjectTodoRecord {
        id: id.to_string(),
        milestone_id: milestone_id.map(str::to_string),
        title: format!("todo {id}"),
        draft: format!("draft {id}"),
        plan_md: None,
        status: ProjectTodoStatus::Draft,
        agent: "act".to_string(),
        active_session_id: None,
        created_at,
        updated_at: created_at,
    }
}

async fn run(store: &dyn ProjectStore, id: &str, todo_id: &str, created_at: i64) {
    let version = store.next_todo_version(todo_id).await.unwrap();
    store
        .create_todo_run(&ProjectTodoRunRecord {
            id: id.to_string(),
            todo_id: todo_id.to_string(),
            kind: ProjectTodoRunKind::Plan,
            version,
            plan_md: Some("plan".to_string()),
            output_md: None,
            agent: "plan".to_string(),
            session_id: Some(format!("sess-{id}")),
            status: ProjectTodoRunStatus::Running,
            started_at: created_at,
            finished_at: None,
            created_at,
        })
        .await
        .unwrap();
}

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
async fn milestone_crud_and_goal_filter() {
    let (_dir, _store, p) = fresh().await;
    p.create_goal(&goal("g1", 0, 1)).await.unwrap();
    p.create_goal(&goal("g2", 0, 2)).await.unwrap();
    p.create_milestone(&milestone("m1", "g1", 2, 10))
        .await
        .unwrap();
    p.create_milestone(&milestone("m2", "g1", 1, 20))
        .await
        .unwrap();
    p.create_milestone(&milestone("m3", "g2", 0, 30))
        .await
        .unwrap();

    let for_g1 = p.list_milestones(Some("g1")).await.unwrap();
    assert_eq!(
        for_g1.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["m2", "m1"],
        "filtered by goal and ordered by sort_key"
    );
    assert_eq!(p.list_milestones(None).await.unwrap().len(), 3);

    assert!(p
        .patch_milestone(
            "m1",
            &ProjectMilestonePatch {
                title: Some("renamed".to_string()),
                status: Some(ProjectMilestoneStatus::InProgress),
                ..Default::default()
            },
            77,
        )
        .await
        .unwrap());
    let m1 = p.list_milestones(Some("g1")).await.unwrap()[1].clone();
    assert_eq!(m1.title, "renamed");
    assert_eq!(m1.status, ProjectMilestoneStatus::InProgress);
    assert_eq!(m1.updated_at, 77);
    assert!(!m1.status.is_terminal());
}

#[tokio::test]
async fn todo_patch_semantics_including_clear_to_null() {
    let (_dir, _store, p) = fresh().await;
    p.create_goal(&goal("g1", 0, 1)).await.unwrap();
    p.create_milestone(&milestone("m1", "g1", 0, 2))
        .await
        .unwrap();
    p.create_todo(&todo("t1", Some("m1"), 3)).await.unwrap();
    p.create_todo(&todo("t2", None, 4)).await.unwrap();

    // Backlog + milestone todos are both covered by the unfiltered list.
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
                milestone_id: Some(None), // back to the backlog
                ..Default::default()
            },
            60,
        )
        .await
        .unwrap());
    let t1 = p.get_todo("t1").await.unwrap().unwrap();
    assert_eq!(t1.plan_md, None, "Some(None) clears plan_md to NULL");
    assert_eq!(t1.active_session_id, None);
    assert_eq!(t1.milestone_id, None, "Some(None) clears milestone_id");
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
async fn delete_goal_cascades_milestone_todo_and_runs() {
    let (_dir, _store, p) = fresh().await;
    p.create_goal(&goal("g1", 0, 1)).await.unwrap();
    p.create_milestone(&milestone("m1", "g1", 0, 2))
        .await
        .unwrap();
    p.create_todo(&todo("t1", Some("m1"), 3)).await.unwrap();
    run(p.as_ref(), "r1", "t1", 4).await;

    assert!(p.delete_goal("g1").await.unwrap());
    assert!(p.list_goals().await.unwrap().is_empty());
    assert!(p.list_milestones(None).await.unwrap().is_empty());
    assert!(p.list_todos(None).await.unwrap().is_empty());
    assert!(p.list_todo_runs("t1").await.unwrap().is_empty());
    assert!(p.get_todo("t1").await.unwrap().is_none());
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
async fn delete_milestone_deletes_its_todos_not_reparents() {
    let (_dir, _store, p) = fresh().await;
    p.create_goal(&goal("g1", 0, 1)).await.unwrap();
    p.create_milestone(&milestone("m1", "g1", 0, 2))
        .await
        .unwrap();
    p.create_milestone(&milestone("m2", "g1", 1, 3))
        .await
        .unwrap();
    p.create_todo(&todo("t1", Some("m1"), 4)).await.unwrap();
    p.create_todo(&todo("t2", Some("m2"), 5)).await.unwrap();
    run(p.as_ref(), "r1", "t1", 6).await;

    assert!(p.delete_milestone("m1").await.unwrap());
    // m1's todo AND its runs are gone — nothing is resurrected as backlog.
    assert!(p.get_todo("t1").await.unwrap().is_none());
    assert!(p.list_todo_runs("t1").await.unwrap().is_empty());
    // m2 and its todo are untouched.
    let remaining = p.list_todos(None).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, "t2");
    assert_eq!(p.list_milestones(Some("g1")).await.unwrap().len(), 1);
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
    assert_eq!(v, 18, "schema_version must be latest (18) after reopen");

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

/// Compile-level: the concrete store coerces to the trait object upper
/// layers hold (`Arc<dyn ProjectStore>`).
#[allow(dead_code)]
fn libsql_store_coerces_to_project_store(store: Arc<LibsqlStore>) -> Arc<dyn ProjectStore> {
    store
}
