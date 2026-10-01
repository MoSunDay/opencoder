//! Tag lifecycle and rollback over a real temporary SQLite store.
use opencoder_store::project::ProjectTag;
use opencoder_store::{
    LibsqlStore, ProjectGoalRecord, ProjectGoalStatus, ProjectInitiativePatch,
    ProjectInitiativeRecord, ProjectInitiativeStatus, ProjectStore, ProjectTodoPatch,
    ProjectTodoRecord,
};

fn initiative(id: &str, goal: Option<&str>) -> ProjectInitiativeRecord {
    ProjectInitiativeRecord {
        id: id.into(),
        goal_id: goal.map(str::to_owned),
        title: id.into(),
        detail_md: None,
        status: ProjectInitiativeStatus::Planned,
        sort: 0,
        created_at: 1,
        updated_at: 1,
    }
}
fn todo(id: &str, initiative: Option<&str>) -> ProjectTodoRecord {
    serde_json::from_value(serde_json::json!({"id":id,"initiative_id":initiative,"title":id,"draft":"说明","status":"draft","board_status":"todo","agent":"act","created_at":1,"updated_at":1})).unwrap()
}
fn tag(id: &str, scope: &str, owner: &str, name: &str) -> ProjectTag {
    ProjectTag {
        id: id.into(),
        scope_type: scope.into(),
        scope_id: owner.into(),
        name: name.into(),
    }
}
async fn setup() -> LibsqlStore {
    let s = LibsqlStore::open_memory().await.unwrap();
    for id in ["p", "q"] {
        s.create_goal(&ProjectGoalRecord {
            id: id.into(),
            title: id.into(),
            detail_md: None,
            status: ProjectGoalStatus::Active,
            sort: 0,
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    }
    s.create_initiative(&initiative("i", Some("p")))
        .await
        .unwrap();
    s.create_initiative(&initiative("j", None)).await.unwrap();
    s
}
#[tokio::test]
async fn local_override_remaps_existing_todos_and_delete_restores_project_tag() {
    let s = setup().await;
    s.write_tag(&tag("p1", "project", "p", "前端"))
        .await
        .unwrap();
    s.write_tag(&tag("p2", "project", "p", "重点"))
        .await
        .unwrap();
    s.create_todo_tagged(&todo("t", Some("i")), &["p1".into(), "p2".into()])
        .await
        .unwrap();
    s.write_tag(&tag("i1", "initiative", "i", "前端"))
        .await
        .unwrap();
    assert_eq!(
        s.list_todo_tags()
            .await
            .unwrap()
            .iter()
            .map(|t| t.tag_id.as_str())
            .collect::<Vec<_>>(),
        ["i1", "p2"]
    );
    s.delete_tag("i1").await.unwrap();
    assert_eq!(
        s.list_todo_tags()
            .await
            .unwrap()
            .iter()
            .map(|t| t.tag_id.as_str())
            .collect::<Vec<_>>(),
        ["p1", "p2"]
    );
    s.delete_tag("p1").await.unwrap();
    assert_eq!(s.list_todo_tags().await.unwrap()[0].tag_id, "p2");
}
#[tokio::test]
async fn invalid_selection_rolls_back_todo_creation_and_edits() {
    let s = setup().await;
    s.write_tag(&tag("other", "initiative", "j", "其他"))
        .await
        .unwrap();
    assert!(s
        .create_todo_tagged(&todo("new", Some("i")), &["other".into()])
        .await
        .is_err());
    assert!(s.get_todo("new").await.unwrap().is_none());
    s.create_todo(&todo("t", Some("i"))).await.unwrap();
    assert!(s
        .patch_todo_tagged(
            "t",
            &ProjectTodoPatch {
                title: Some("不能保存".into()),
                ..Default::default()
            },
            Some(&["other".into()]),
            2
        )
        .await
        .is_err());
    assert_eq!(s.get_todo("t").await.unwrap().unwrap().title, "t");
    assert!(s.list_todo_tags().await.unwrap().is_empty());
}
#[tokio::test]
async fn rename_duplicate_and_scope_changes_keep_catalog_consistent() {
    let s = setup().await;
    s.write_tag(&tag("p1", "project", "p", "模块"))
        .await
        .unwrap();
    s.write_tag(&tag("q1", "project", "q", "模块"))
        .await
        .unwrap();
    assert!(s
        .write_tag(&tag("duplicate", "project", "p", "模块"))
        .await
        .is_err());
    s.create_todo_tagged(&todo("t", Some("i")), &["p1".into(), "p1".into()])
        .await
        .unwrap();
    assert_eq!(s.list_todo_tags().await.unwrap().len(), 1);
    s.patch_initiative(
        "i",
        &ProjectInitiativePatch {
            goal_id: Some(Some("q".into())),
            ..Default::default()
        },
        2,
    )
    .await
    .unwrap();
    assert_eq!(s.list_todo_tags().await.unwrap()[0].tag_id, "q1");
    s.write_tag(&tag("q1", "project", "q", "新模块"))
        .await
        .unwrap();
    assert_eq!(s.list_todo_tags().await.unwrap()[0].tag_id, "q1");
    s.delete_goal("q").await.unwrap();
    assert!(s.list_todo_tags().await.unwrap().is_empty());
    assert!(s.get_todo("t").await.unwrap().is_some());
    assert!(s
        .list_initiatives(None)
        .await
        .unwrap()
        .iter()
        .find(|i| i.id == "i")
        .unwrap()
        .goal_id
        .is_none());
}
#[tokio::test]
async fn todo_unlink_and_deletion_remove_tag_links() {
    let s = setup().await;
    s.write_tag(&tag("i1", "initiative", "i", "模块"))
        .await
        .unwrap();
    s.create_todo_tagged(&todo("t", Some("i")), &["i1".into()])
        .await
        .unwrap();
    s.patch_todo(
        "t",
        &ProjectTodoPatch {
            initiative_id: Some(None),
            ..Default::default()
        },
        2,
    )
    .await
    .unwrap();
    assert!(s.list_todo_tags().await.unwrap().is_empty());
    s.create_todo_tagged(&todo("other", Some("i")), &["i1".into()])
        .await
        .unwrap();
    s.delete_todo("other").await.unwrap();
    assert!(s.list_todo_tags().await.unwrap().is_empty());
    assert!(s.get_todo("t").await.unwrap().is_some());
}
#[tokio::test]
async fn v31_migration_removes_legacy_containers_but_preserves_todo_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let s = LibsqlStore::open(&path).await.unwrap();
    s.create_initiative(&initiative("keep", None))
        .await
        .unwrap();
    s.create_initiative(&initiative("legacy", None))
        .await
        .unwrap();
    s.create_todo(&todo("a", Some("keep"))).await.unwrap();
    s.create_todo(&todo("b", Some("legacy"))).await.unwrap();
    drop(s);
    let db = libsql::Builder::new_local(&path).build().await.unwrap();
    let c = db.connect().unwrap();
    c.execute_batch(
        "DROP TABLE project_todo_tags;
         DROP TABLE project_tags;
         DROP INDEX idx_project_initiatives_goal;
         DROP INDEX idx_project_todos_initiative;",
    )
    .await
    .unwrap();
    c.execute(
        "ALTER TABLE project_initiatives RENAME TO project_milestones",
        (),
    )
    .await
    .unwrap();
    c.execute(
        "ALTER TABLE project_milestones ADD COLUMN kind TEXT NOT NULL DEFAULT 'initiative'",
        (),
    )
    .await
    .unwrap();
    c.execute(
        "UPDATE project_milestones SET kind='milestone' WHERE id='legacy'",
        (),
    )
    .await
    .unwrap();
    c.execute(
        "ALTER TABLE project_todos RENAME COLUMN initiative_id TO milestone_id",
        (),
    )
    .await
    .unwrap();
    c.execute("UPDATE schema_version SET version=31", ())
        .await
        .unwrap();
    c.execute_batch(
        "CREATE INDEX idx_project_milestones_goal ON project_milestones(goal_id);
         CREATE INDEX idx_project_todos_milestone ON project_todos(milestone_id);",
    )
    .await
    .unwrap();
    drop(c);
    drop(db);
    for _ in 0..2 {
        let s = LibsqlStore::open(&path).await.unwrap();
        assert_eq!(
            s.list_initiatives(None)
                .await
                .unwrap()
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>(),
            ["keep"]
        );
        let a = s.get_todo("a").await.unwrap().unwrap();
        let b = s.get_todo("b").await.unwrap().unwrap();
        assert_eq!(a.initiative_id.as_deref(), Some("keep"));
        assert!(b.initiative_id.is_none());
        assert_eq!(b.draft, "说明");
        assert_eq!(b.board_status, "todo");
    }
}

#[tokio::test]
async fn tag_scope_rejects_missing_owner_and_cannot_move_existing_definition() {
    let s = setup().await;
    assert!(s
        .write_tag(&tag("missing", "project", "gone", "模块"))
        .await
        .is_err());
    assert!(s
        .write_tag(&tag("invalid", "todo", "i", "模块"))
        .await
        .is_err());
    assert!(s.list_tags().await.unwrap().is_empty());
    s.write_tag(&tag("t", "project", "p", "模块"))
        .await
        .unwrap();
    assert!(s
        .write_tag(&tag("t", "project", "q", "模块"))
        .await
        .is_err());
    assert_eq!(
        s.list_tags().await.unwrap(),
        [tag("t", "project", "p", "模块")]
    );
}
