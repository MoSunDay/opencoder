//! Catalog CRUD, patch semantics, executor dimension and deletion cascades.
use super::*;

#[tokio::test]
async fn goal_crud_patch_and_missing_id() {
    let p = mem_iface().await;
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
            &opencoder_store::ProjectGoalPatch {
                current_summary: None,
                people: Default::default(),
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

    // Duplicate id is a constraint error, not an overwrite.
    assert!(p.create_goal(&goal("g1", 0, 1)).await.is_err());

    // Missing ids: false, not an error.
    assert!(!p.patch_goal("nope", &Default::default(), 1).await.unwrap());
    assert!(!p.delete_goal("nope").await.unwrap());
}

#[tokio::test]
async fn initiative_crud_and_goal_filter() {
    let p = mem_iface().await;
    p.create_goal(&goal("g1", 0, 1)).await.unwrap();
    p.create_goal(&goal("g2", 0, 2)).await.unwrap();
    p.create_initiative(&initiative("m1", Some("g1"), 2, 10))
        .await
        .unwrap();
    p.create_initiative(&initiative("m2", Some("g1"), 1, 20))
        .await
        .unwrap();
    p.create_initiative(&initiative("m3", Some("g2"), 0, 30))
        .await
        .unwrap();

    let for_g1 = p.list_initiatives(Some("g1")).await.unwrap();
    assert_eq!(
        for_g1.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["m2", "m1"],
        "filtered by goal and ordered by sort"
    );
    assert_eq!(p.list_initiatives(None).await.unwrap().len(), 3);
    assert!(p
        .create_initiative(&initiative("m1", Some("g1"), 0, 1))
        .await
        .is_err());

    assert!(p
        .patch_initiative(
            "m1",
            &opencoder_store::ProjectInitiativePatch {
                people: Default::default(),
                title: Some("renamed".to_string()),
                sort: Some(7),
                ..Default::default()
            },
            77,
        )
        .await
        .unwrap());
    let m1 = p.list_initiatives(Some("g1")).await.unwrap()[1].clone();
    assert_eq!(m1.sort, 7);
    assert_eq!(m1.title, "renamed");
    assert_eq!(m1.updated_at, 77);
    assert!(serde_json::to_value(&m1).unwrap().get("status").is_none());
    assert!(!p
        .patch_initiative("missing", &Default::default(), 1)
        .await
        .unwrap());
}

#[tokio::test]
async fn todo_patch_semantics_including_clear_to_null() {
    let p = mem_iface().await;
    p.create_goal(&goal("g1", 0, 1)).await.unwrap();
    p.create_initiative(&initiative("m1", Some("g1"), 0, 2))
        .await
        .unwrap();
    p.create_todo(&todo("t1", Some("m1"), 3)).await.unwrap();
    p.create_todo(&todo("t2", None, 4)).await.unwrap();

    // Backlog + initiative todos are both covered by the unfiltered list.
    assert_eq!(p.list_todos(None).await.unwrap().len(), 2);
    let for_m1 = p.list_todos(Some("m1")).await.unwrap();
    assert_eq!(for_m1.len(), 1);
    assert_eq!(for_m1[0].id, "t1");
    assert!(p.create_todo(&todo("t1", None, 9)).await.is_err());

    // Set plan_md + active_session_id, then clear them to NULL.
    assert!(p
        .patch_todo(
            "t1",
            &opencoder_store::ProjectTodoPatch {
                people: Default::default(),
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
            &opencoder_store::ProjectTodoPatch {
                people: Default::default(),
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
        .patch_todo("missing", &Default::default(), 1)
        .await
        .unwrap());
}

#[tokio::test]
async fn executor_dimension_round_trips_and_fails_closed() {
    let store = mem().await;
    let p = store.clone();
    let mut team = todo("t-team", None, 1);
    team.executor_kind = ProjectExecutorKind::Team;
    team.executor_ref = Some("feature-team".to_string());
    team.executor_spec = Some(r#"{"name":"feature-team"}"#.to_string());
    p.create_todo(&team).await.unwrap();

    let back = p.get_todo("t-team").await.unwrap().unwrap();
    assert_eq!(back.executor_kind, ProjectExecutorKind::Team);
    assert_eq!(back.executor_ref.as_deref(), Some("feature-team"));
    assert_eq!(
        back.executor_spec.as_deref(),
        Some(r#"{"name":"feature-team"}"#)
    );

    for (value, expected) in [
        (
            Some("other-team".to_string()),
            Some("other-team".to_string()),
        ),
        (None, None),
    ] {
        p.patch_todo(
            "t-team",
            &opencoder_store::ProjectTodoPatch {
                executor_ref: Some(value),
                ..Default::default()
            },
            10,
        )
        .await
        .unwrap();
        assert_eq!(
            p.get_todo("t-team").await.unwrap().unwrap().executor_ref,
            expected
        );
    }

    let dag_run = ProjectTodoRunRecord {
        input_snapshot: None,
        trace_manifest: None,
        id: "run-dag".to_string(),
        todo_id: "t-team".to_string(),
        kind: ProjectTodoRunKind::Execute,
        version: 1,
        plan_md: None,
        output_md: None,
        agent: "act".to_string(),
        executor_kind: ProjectExecutorKind::Dag,
        capability_id: Some("cap-42".to_string()),
        plan_id: Some("plan-42".to_string()),
        output_ref: Some("/workflow/run-9/step-2/".to_string()),
        session_id: None,
        status: ProjectTodoRunStatus::Running,
        started_at: 2,
        finished_at: None,
        created_at: 2,
    };
    p.create_todo_run(&dag_run).await.unwrap();
    let run_back = p.get_todo_run("run-dag").await.unwrap().unwrap();
    assert_eq!(run_back.executor_kind, ProjectExecutorKind::Dag);
    assert_eq!(run_back.capability_id.as_deref(), Some("cap-42"));
    assert_eq!(run_back.plan_id.as_deref(), Some("plan-42"));
    assert_eq!(
        run_back.output_ref.as_deref(),
        Some("/workflow/run-9/step-2/")
    );

    // Unknown executor_kind text fails closed instead of guessing: close the
    // store, corrupt the JSON row through a raw engine handle, reopen.
    // (RocksDB holds the directory lock per handle, so the two cannot be
    // open at once in-process.)
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let s2 = RdbStore::open(&path).await.unwrap();
    s2.create_todo(&todo("t-bad", None, 1)).await.unwrap();
    let row = s2.get_todo("t-bad").await.unwrap().unwrap();
    drop(s2);
    let db = opencoder_store::rdb_store::engine::RdbDb::open(&path).unwrap();
    let mut value = serde_json::to_value(row).unwrap();
    value["executor_kind"] = serde_json::json!("workflow");
    db.write(move |s| {
        let mut batch = rocksdb::WriteBatch::default();
        opencoder_store::rdb_store::tx::put_raw(
            &mut batch,
            b"ptd/t-bad",
            &opencoder_store::rdb_store::kv::encode(&value)?,
        );
        opencoder_store::rdb_store::tx::commit(s, batch)
    })
    .await
    .unwrap();
    drop(db);
    let s2 = RdbStore::open(&path).await.unwrap();
    assert!(s2.get_todo("t-bad").await.is_err());
}

#[tokio::test]
async fn delete_goal_detaches_initiatives_and_preserves_the_rest() {
    let p = mem_iface().await;
    p.create_goal(&goal("g1", 0, 1)).await.unwrap();
    p.create_initiative(&initiative("m1", Some("g1"), 0, 2))
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
    let p = mem_iface().await;
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
    let p = mem_iface().await;
    p.create_goal(&goal("g1", 0, 1)).await.unwrap();
    p.create_initiative(&initiative("m1", Some("g1"), 0, 2))
        .await
        .unwrap();
    p.create_initiative(&initiative("m2", Some("g1"), 1, 3))
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
        &opencoder_store::ProjectTodoPatch {
            people: Default::default(),
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
    assert!(!p.delete_initiative("missing").await.unwrap());
}

#[tokio::test]
async fn board_reorder_moves_once_and_rolls_back_on_unknown_id() {
    let p = mem_iface().await;
    p.create_todo(&todo("a", None, 1)).await.unwrap();
    p.create_todo(&todo("b", None, 2)).await.unwrap();
    p.reorder_todos(None, "todo", &["b".into(), "a".into()], 5)
        .await
        .unwrap();
    let rows = p.list_todos(None).await.unwrap();
    assert_eq!(
        rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert!(rows.iter().all(|row| row.board_status == "todo"));
    assert!(p
        .reorder_todos(None, "done", &["a".into(), "missing".into()], 6)
        .await
        .is_err());
    assert_eq!(p.get_todo("a").await.unwrap().unwrap().board_status, "todo");
    p.create_todo(&todo("foreign", Some("other"), 3))
        .await
        .unwrap();
    assert!(p
        .reorder_todos(None, "done", &["a".into(), "foreign".into()], 7)
        .await
        .is_err());
    assert_eq!(p.get_todo("a").await.unwrap().unwrap().board_status, "todo");
    assert_eq!(
        p.get_todo("foreign").await.unwrap().unwrap().board_status,
        "backlog"
    );
}
