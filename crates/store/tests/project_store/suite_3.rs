use super::*;

#[tokio::test]
async fn executor_dimension_round_trips() {
    let (_dir, _store, iface) = fresh().await;

    // Todo side: team executor with a ref and an inline spec.
    let mut team = todo("t-team", None, 1);
    team.executor_kind = ProjectExecutorKind::Team;
    team.executor_ref = Some("feature-team".to_string());
    team.executor_spec = Some(r#"{"name":"feature-team"}"#.to_string());
    iface.create_todo(&team).await.unwrap();

    let back = iface.get_todo("t-team").await.unwrap().unwrap();
    assert_eq!(back.executor_kind, ProjectExecutorKind::Team);
    assert_eq!(back.executor_ref.as_deref(), Some("feature-team"));
    assert_eq!(
        back.executor_spec.as_deref(),
        Some(r#"{"name":"feature-team"}"#)
    );

    // Patching executor_ref through Option<Option<String>> (set + clear).
    iface
        .patch_todo(
            "t-team",
            &ProjectTodoPatch {
                executor_ref: Some(Some("other-team".to_string())),
                ..Default::default()
            },
            10,
        )
        .await
        .unwrap();
    assert_eq!(
        iface
            .get_todo("t-team")
            .await
            .unwrap()
            .unwrap()
            .executor_ref,
        Some("other-team".to_string())
    );
    iface
        .patch_todo(
            "t-team",
            &ProjectTodoPatch {
                executor_ref: Some(None),
                ..Default::default()
            },
            11,
        )
        .await
        .unwrap();
    assert_eq!(
        iface
            .get_todo("t-team")
            .await
            .unwrap()
            .unwrap()
            .executor_ref,
        None
    );

    // Run side: dag run with brain provenance and an artifact root.
    let mut dag_run = ProjectTodoRunRecord {
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
        capability_id: Some("cap-brain".to_string()),
        plan_id: Some("plan-42".to_string()),
        output_ref: Some("/workflow/run-9/step-2/".to_string()),
        session_id: None,
        status: ProjectTodoRunStatus::Running,
        started_at: 2,
        finished_at: None,
        created_at: 2,
    };
    assert!(iface
        .claim_todo_running_with_run(&dag_run, 2)
        .await
        .unwrap());
    dag_run.executor_kind = ProjectExecutorKind::Brain;
    assert!(!iface
        .claim_todo_running_with_run(&dag_run, 3)
        .await
        .unwrap());

    let run_back = iface.get_todo_run("run-dag").await.unwrap().unwrap();
    assert_eq!(run_back.executor_kind, ProjectExecutorKind::Dag);
    assert_eq!(run_back.capability_id.as_deref(), Some("cap-brain"));
    assert_eq!(run_back.plan_id.as_deref(), Some("plan-42"));
    assert_eq!(
        run_back.output_ref.as_deref(),
        Some("/workflow/run-9/step-2/")
    );

    // Unknown executor_kind text fails closed instead of guessing.
    {
        let store2 = LibsqlStore::open(tempfile::tempdir().unwrap().path().join("x.db"))
            .await
            .unwrap();
        store2.create_todo(&todo("t-bad", None, 1)).await.unwrap();
        let conn = store2.conn().await.unwrap();
        conn.execute(
            "UPDATE project_todos SET executor_kind = 'workflow' WHERE id = 't-bad'",
            (),
        )
        .await
        .unwrap();
        assert!(store2.get_todo("t-bad").await.is_err());
    }
}

#[tokio::test]
async fn board_reorder_moves_once_and_rolls_back_on_unknown_id() {
    let (_dir, _store, p) = fresh().await;
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
