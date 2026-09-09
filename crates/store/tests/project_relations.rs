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
    let view = opencoder_store::project::overview::overview(&[], &rows, &todos);
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
