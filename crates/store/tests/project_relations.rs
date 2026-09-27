//! All mutations in this suite target freshly-created temporary databases.
use opencoder_store::{
    LibsqlStore, ProjectMilestonePatch, ProjectMilestoneRecord, ProjectMilestoneStatus,
    ProjectStore, ProjectTodoPatch, ProjectTodoRecord,
};

fn todo(id: &str) -> ProjectTodoRecord {
    serde_json::from_value(serde_json::json!({
        "id":id,"milestone_id":null,"title":"任务","draft":"不能丢的正文 界",
        "plan_md":"历史计划","status":"planned","agent":"act",
        "active_session_id":"session-retained","created_at":7,"updated_at":8,
    }))
    .unwrap()
}

#[tokio::test]
async fn milestones_and_initiatives_remain_distinct_and_both_hold_todos() {
    let store = LibsqlStore::open_memory().await.unwrap();
    let group = ProjectMilestoneRecord {
        id: "m1".into(),
        goal_id: None,
        title: "里程碑".into(),
        detail_md: None,
        status: ProjectMilestoneStatus::Planned,
        sort: 0,
        created_at: 1,
        updated_at: 1,
    };
    store.create_milestone(&group).await.unwrap();
    let initiative = ProjectMilestoneRecord {
        id: "i1".into(),
        title: "专项".into(),
        ..group
    };
    store.create_initiative(&initiative).await.unwrap();
    assert_eq!(
        store
            .list_milestones(None)
            .await
            .unwrap()
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["m1"]
    );
    assert_eq!(
        store
            .list_initiatives(None)
            .await
            .unwrap()
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["i1"]
    );
    assert!(!store
        .patch_initiative(
            "m1",
            &ProjectMilestonePatch {
                title: Some("错误类型".into()),
                ..Default::default()
            },
            2,
        )
        .await
        .unwrap());
    store
        .create_todo(&ProjectTodoRecord {
            milestone_id: Some("i1".into()),
            ..todo("t1")
        })
        .await
        .unwrap();
    assert!(store.delete_milestone("i1").await.unwrap() == false);
    assert!(store.delete_initiative("i1").await.is_err());
    let todos = store
        .list_todos(None)
        .await
        .unwrap()
        .iter()
        .map(|item| serde_json::to_value(item).unwrap())
        .collect::<Vec<_>>();
    let view = opencoder_store::project::overview::overview(
        &[],
        &store.list_milestones(None).await.unwrap(),
        &store.list_initiatives(None).await.unwrap(),
        &todos,
    );
    assert_eq!(view["standalone_initiatives"][0]["todos"][0]["id"], "t1");
    assert_eq!(
        view["standalone_milestones"][0]["todos"],
        serde_json::json!([])
    );
}

#[tokio::test]
async fn standalone_milestone_and_optional_todo_association_roundtrip() {
    let store = LibsqlStore::open_memory().await.unwrap();
    let milestone = ProjectMilestoneRecord {
        id: "m".into(),
        goal_id: None,
        title: "专项".into(),
        detail_md: Some("正文".into()),
        status: ProjectMilestoneStatus::Planned,
        sort: 0,
        created_at: 1,
        updated_at: 1,
    };
    store.create_milestone(&milestone).await.unwrap();
    store.create_todo(&todo("t")).await.unwrap();
    store
        .patch_todo(
            "t",
            &ProjectTodoPatch {
                milestone_id: Some(Some("m".into())),
                ..Default::default()
            },
            10,
        )
        .await
        .unwrap();
    let rows = store.list_milestones(None).await.unwrap();
    assert_eq!(rows[0].goal_id, None);
    store
        .patch_milestone(
            "m",
            &ProjectMilestonePatch {
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
    let view = opencoder_store::project::overview::overview(&[], &rows, &[], &todos);
    assert_eq!(view["standalone_milestones"][0]["todos"][0]["id"], "t");
    assert_eq!(view["backlog"].as_array().unwrap().len(), 0);
    store
        .patch_todo(
            "t",
            &ProjectTodoPatch {
                milestone_id: Some(None),
                ..Default::default()
            },
            12,
        )
        .await
        .unwrap();
    assert!(store.delete_milestone("m").await.unwrap());
    assert_eq!(
        store.get_todo("t").await.unwrap().unwrap().draft,
        todo("t").draft
    );
}

#[tokio::test]
async fn todo_execution_links_store_only_ids_and_cascade_on_delete() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store.create_todo(&todo("linked")).await.unwrap();
    store
        .link_todo_execution("linked", "agent-a")
        .await
        .unwrap();
    store
        .link_todo_execution("linked", "agent-a")
        .await
        .unwrap();
    store
        .link_todo_execution("linked", "brain-b")
        .await
        .unwrap();
    let ids = store.list_todo_execution_ids("linked").await.unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"agent-a".to_string()));
    assert!(ids.contains(&"brain-b".to_string()));
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
        .list_todo_execution_ids("linked")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn v22_upgrade_preserves_data_and_classifies_only_legacy_backlog_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    {
        let store = LibsqlStore::open(&path).await.unwrap();
        store.create_todo(&todo("old")).await.unwrap();
        let conn = store.conn().await.unwrap();
        // Fixture represents the historical NOT NULL schema with real content.
        conn.execute_batch("DROP TABLE project_milestones;
            CREATE TABLE project_milestones (id TEXT PRIMARY KEY,goal_id TEXT NOT NULL,title TEXT NOT NULL,detail_md TEXT,status TEXT NOT NULL,sort_key INTEGER NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
            INSERT INTO project_milestones VALUES('existing','g','既有专项','# 原始正文','done',9,3,4);
            UPDATE schema_version SET version=22;").await.unwrap();
    }
    let store = LibsqlStore::open(&path).await.unwrap();
    let old = store.get_todo("old").await.unwrap().unwrap();
    let mut expected = todo("old");
    expected.milestone_id = old.milestone_id.clone();
    assert!(old.milestone_id.is_some());
    assert_eq!(
        serde_json::to_value(&old).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    let milestones = store.list_milestones(None).await.unwrap();
    assert_eq!(milestones.len(), 2);
    let existing = milestones.iter().find(|m| m.id == "existing").unwrap();
    assert_eq!(existing.goal_id.as_deref(), Some("g"));
    assert_eq!(existing.detail_md.as_deref(), Some("# 原始正文"));
    assert_eq!(existing.updated_at, 4);
    assert!(milestones
        .iter()
        .any(|m| m.title == "待归类" && m.goal_id.is_none()));
    store.create_todo(&todo("new")).await.unwrap();
    drop(store);
    let reopened = LibsqlStore::open(&path).await.unwrap();
    assert_eq!(reopened.list_milestones(None).await.unwrap().len(), 2);
    assert_eq!(
        reopened
            .get_todo("old")
            .await
            .unwrap()
            .unwrap()
            .milestone_id,
        old.milestone_id
    );
    assert!(reopened
        .get_todo("new")
        .await
        .unwrap()
        .unwrap()
        .milestone_id
        .is_none());
}
