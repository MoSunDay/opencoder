//! All mutations in this suite target freshly-created temporary databases.
use opencoder_store::project::ProjectAssignment;
use opencoder_store::{
    LibsqlStore, ProjectInitiativePatch, ProjectInitiativeRecord, ProjectInitiativeStatus,
    ProjectStore, ProjectTodoPatch, ProjectTodoRecord,
};

fn assignment(todo_id: &str, execution_id: &str, kind: &str) -> ProjectAssignment {
    ProjectAssignment {
        todo_id: todo_id.into(),
        execution_id: execution_id.into(),
        kind: kind.into(),
        name: execution_id.into(),
        created_at: 10,
        capability_id: Some(format!("capability-{kind}")),
    }
}

fn todo(id: &str) -> ProjectTodoRecord {
    serde_json::from_value(serde_json::json!({
        "id":id,"initiative_id":null,"title":"任务","draft":"不能丢的正文 界",
        "plan_md":"历史计划","status":"planned","agent":"act",
        "active_session_id":"session-retained","created_at":7,"updated_at":8,
    }))
    .unwrap()
}

#[tokio::test]
async fn initiatives_are_the_only_containers_and_nonempty_ones_reject_deletion() {
    let store = LibsqlStore::open_memory().await.unwrap();
    let group = ProjectInitiativeRecord {
        id: "i1".into(),
        goal_id: None,
        title: "专项".into(),
        detail_md: None,
        status: ProjectInitiativeStatus::Planned,
        sort: 0,
        created_at: 1,
        updated_at: 1,
    };
    store.create_initiative(&group).await.unwrap();
    store
        .create_todo(&ProjectTodoRecord {
            initiative_id: Some("i1".into()),
            ..todo("t1")
        })
        .await
        .unwrap();
    assert!(store.delete_initiative("i1").await.is_err());
    assert!(!store.delete_initiative("missing").await.unwrap());
    let view = opencoder_store::project::overview::load(&store)
        .await
        .unwrap();
    assert_eq!(view["standalone_initiatives"][0]["todos"][0]["id"], "t1");
    assert!(view.get("standalone_milestones").is_none());
    store.delete_todo("t1").await.unwrap();
    assert!(store.delete_initiative("i1").await.unwrap());
}

#[tokio::test]
async fn standalone_initiative_and_optional_todo_association_roundtrip() {
    let store = LibsqlStore::open_memory().await.unwrap();
    let initiative = ProjectInitiativeRecord {
        id: "m".into(),
        goal_id: None,
        title: "专项".into(),
        detail_md: Some("正文".into()),
        status: ProjectInitiativeStatus::Planned,
        sort: 0,
        created_at: 1,
        updated_at: 1,
    };
    store.create_initiative(&initiative).await.unwrap();
    store.create_todo(&todo("t")).await.unwrap();
    store
        .patch_todo(
            "t",
            &ProjectTodoPatch {
                initiative_id: Some(Some("m".into())),
                ..Default::default()
            },
            10,
        )
        .await
        .unwrap();
    let rows = store.list_initiatives(None).await.unwrap();
    assert_eq!(rows[0].goal_id, None);
    store
        .patch_initiative(
            "m",
            &ProjectInitiativePatch {
                goal_id: Some(None),
                ..Default::default()
            },
            11,
        )
        .await
        .unwrap();
    let todos = store
        .list_todos(None)
        .await
        .unwrap()
        .iter()
        .map(|v| serde_json::to_value(v).unwrap())
        .collect::<Vec<_>>();
    let view = opencoder_store::project::overview::overview(&[], &rows, &todos);
    assert_eq!(view["standalone_initiatives"][0]["todos"][0]["id"], "t");
    assert_eq!(view["backlog"].as_array().unwrap().len(), 0);
    store
        .patch_todo(
            "t",
            &ProjectTodoPatch {
                initiative_id: Some(None),
                ..Default::default()
            },
            12,
        )
        .await
        .unwrap();
    assert!(store.delete_initiative("m").await.unwrap());
    assert_eq!(
        store.get_todo("t").await.unwrap().unwrap().draft,
        todo("t").draft
    );
}

#[tokio::test]
async fn todo_assignments_only_keep_references_and_cascade_links_on_delete() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store.create_todo(&todo("linked")).await.unwrap();
    for (id, kind) in [
        ("agent-a", "agent"),
        ("agent-a", "agent"),
        ("brain-b", "brain"),
    ] {
        store
            .link_todo_execution(&assignment("linked", id, kind))
            .await
            .unwrap();
    }
    let rows = store.list_todo_assignments("linked").await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].kind, "brain");
    assert_eq!(rows[0].capability_id.as_deref(), Some("capability-brain"));
    let value = serde_json::to_value(&rows[0]).unwrap();
    assert!(value.get("result_md").is_none());
    assert!(value.get("sync_state").is_none());
    let latest = store.latest_todo_assignments().await.unwrap();
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].execution_id, "brain-b");
    let mut retry = assignment("linked", "agent-new", "agent");
    retry.created_at = 11;
    store.link_todo_execution(&retry).await.unwrap();
    assert_eq!(
        store.latest_todo_assignments().await.unwrap()[0].execution_id,
        "agent-new"
    );
    assert!(store
        .unlink_todo_execution("linked", "agent-a")
        .await
        .unwrap());
    assert!(!store
        .unlink_todo_execution("linked", "agent-a")
        .await
        .unwrap());
    store.delete_todo("linked").await.unwrap();
    assert!(store
        .list_todo_assignments("linked")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn all_links_remain_available_without_background_scanning() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store.create_todo(&todo("linked")).await.unwrap();
    for index in 0..30 {
        store
            .link_todo_execution(&assignment("linked", &format!("agent-{index:02}"), "agent"))
            .await
            .unwrap();
    }
    let rows = store.list_todo_assignments("linked").await.unwrap();
    assert_eq!(rows.len(), 30);
    assert_eq!(rows[0].execution_id, "agent-29");
    assert_eq!(rows[29].execution_id, "agent-00");
}

#[tokio::test]
async fn latest_assignment_is_selected_independently_for_each_todo() {
    let store = LibsqlStore::open_memory().await.unwrap();
    for todo_id in ["a", "a-older"] {
        store.create_todo(&todo(todo_id)).await.unwrap();
        store
            .link_todo_execution(&assignment(todo_id, "agent-1", "agent"))
            .await
            .unwrap();
    }
    assert_eq!(store.latest_todo_assignments().await.unwrap().len(), 2);
    store.unlink_todo_execution("a", "agent-1").await.unwrap();
    assert_eq!(
        store.latest_todo_assignments().await.unwrap()[0].todo_id,
        "a-older"
    );
}

#[tokio::test]
async fn v30_execution_links_upgrade_without_losing_ids() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("links.db");
    {
        let store = LibsqlStore::open(&path).await.unwrap();
        store.create_todo(&todo("old-link")).await.unwrap();
        store.conn().await.unwrap().execute_batch("DROP TABLE project_todo_executions;
            CREATE TABLE project_todo_executions (todo_id TEXT NOT NULL, execution_id TEXT NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY (todo_id, execution_id));
            INSERT INTO project_todo_executions VALUES ('old-link','agent-old',1);
            UPDATE schema_version SET version=30;").await.unwrap();
    }
    for _ in 0..2 {
        let store = LibsqlStore::open(&path).await.unwrap();
        let rows = store.list_todo_assignments("old-link").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].execution_id, "agent-old");
        assert_eq!(rows[0].created_at, 1);
        assert!(rows[0].capability_id.is_none());
    }
}

#[tokio::test]
async fn v32_migration_removes_execution_caches_and_keeps_index_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("links.db");
    {
        let store = LibsqlStore::open(&path).await.unwrap();
        store.create_todo(&todo("linked")).await.unwrap();
        store
            .link_todo_execution(&assignment("linked", "agent-1", "agent"))
            .await
            .unwrap();
        store
            .conn()
            .await
            .unwrap()
            .execute_batch(
                "ALTER TABLE project_todo_executions ADD COLUMN result_md TEXT;
            ALTER TABLE project_todo_executions ADD COLUMN sync_state TEXT;
            UPDATE project_todo_executions SET result_md='obsolete output',sync_state='complete';
            UPDATE schema_version SET version=32;",
            )
            .await
            .unwrap();
    }
    let store = LibsqlStore::open(&path).await.unwrap();
    let row = store
        .list_todo_assignments("linked")
        .await
        .unwrap()
        .remove(0);
    assert_eq!(row.capability_id.as_deref(), Some("capability-agent"));
    assert_eq!(row.execution_id, "agent-1");
    assert_eq!(row.name, "agent-1");
    let conn = store.conn().await.unwrap();
    assert!(conn
        .query("SELECT result_md FROM project_todo_executions", ())
        .await
        .is_err());
    assert!(conn
        .query("SELECT sync_state FROM project_todo_executions", ())
        .await
        .is_err());
    assert_eq!(
        store.get_todo("linked").await.unwrap().unwrap().draft,
        todo("linked").draft
    );
}

#[tokio::test]
async fn v22_upgrade_preserves_todos_and_cleans_retired_milestones() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    {
        let store = LibsqlStore::open(&path).await.unwrap();
        store.create_todo(&todo("old")).await.unwrap();
        let conn = store.conn().await.unwrap();
        conn.execute_batch("ALTER TABLE project_todos RENAME COLUMN initiative_id TO milestone_id;
            CREATE TABLE project_milestones (id TEXT PRIMARY KEY,goal_id TEXT NOT NULL,title TEXT NOT NULL,detail_md TEXT,status TEXT NOT NULL,sort_key INTEGER NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
            INSERT INTO project_milestones VALUES('existing','g','旧里程碑','# 原始正文','done',9,3,4);
            UPDATE schema_version SET version=22;").await.unwrap();
    }
    for _ in 0..2 {
        let store = LibsqlStore::open(&path).await.unwrap();
        assert!(store.list_initiatives(None).await.unwrap().is_empty());
        assert_eq!(
            serde_json::to_value(store.get_todo("old").await.unwrap().unwrap()).unwrap(),
            serde_json::to_value(todo("old")).unwrap()
        );
    }
}
