//! Tag catalog lifecycle: overrides, moves, rollback on invalid selections
//! and link cleanup (ported from the SQL tag suites).
use super::*;
use opencoder_store::project::ProjectTag;

fn tag(id: &str, scope: &str, owner: &str, name: &str) -> ProjectTag {
    ProjectTag {
        id: id.into(),
        scope_type: scope.into(),
        scope_id: owner.into(),
        name: name.into(),
    }
}

async fn setup() -> Arc<dyn ProjectStore> {
    let s = mem_iface().await;
    for id in ["p", "q"] {
        s.create_goal(&goal(id, 0, 1)).await.unwrap();
    }
    s.create_initiative(&initiative("i", Some("p"), 0, 1))
        .await
        .unwrap();
    s.create_initiative(&initiative("j", None, 0, 1))
        .await
        .unwrap();
    s
}

#[tokio::test]
async fn local_override_remaps_existing_todos_and_delete_restores_project_tag() {
    let s = setup().await;
    for (id, name) in [("p1", "前端"), ("p2", "重点")] {
        s.write_tag(&tag(id, "project", "p", name)).await.unwrap();
    }
    s.create_todo_tagged(&todo("t", Some("i"), 1), &["p1".into(), "p2".into()])
        .await
        .unwrap();
    // An initiative tag with the same name as a project tag takes over the
    // link of every in-scope todo.
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
    // Deleting the override restores the project-level link.
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
        .create_todo_tagged(&todo("new", Some("i"), 1), &["other".into()])
        .await
        .is_err());
    assert!(s.get_todo("new").await.unwrap().is_none());
    s.create_todo(&todo("t", Some("i"), 1)).await.unwrap();
    assert!(s
        .patch_todo_tagged(
            "t",
            &ProjectTodoPatch {
                title: Some("不能保存".into()),
                ..tpatch(None)
            },
            Some(&["other".into()]),
            2,
        )
        .await
        .is_err());
    assert_eq!(s.get_todo("t").await.unwrap().unwrap().title, "todo t");
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
    // Duplicate ids in one selection collapse to a single link.
    s.create_todo_tagged(&todo("t", Some("i"), 1), &["p1".into(), "p1".into()])
        .await
        .unwrap();
    assert_eq!(s.list_todo_tags().await.unwrap().len(), 1);
    // Moving the todo's initiative across goals remaps to the new goal's tag.
    s.patch_initiative(
        "i",
        &opencoder_store::ProjectInitiativePatch {
            people: Default::default(),
            goal_id: Some(Some("q".into())),
            ..Default::default()
        },
        2,
    )
    .await
    .unwrap();
    assert_eq!(s.list_todo_tags().await.unwrap()[0].tag_id, "q1");
    // Renaming within the new scope keeps the link.
    s.write_tag(&tag("q1", "project", "q", "新模块"))
        .await
        .unwrap();
    assert_eq!(s.list_todo_tags().await.unwrap()[0].tag_id, "q1");
    // Deleting the goal detaches its todos' links but keeps the todos.
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
    s.create_todo_tagged(&todo("t", Some("i"), 1), &["i1".into()])
        .await
        .unwrap();
    s.patch_todo(
        "t",
        &ProjectTodoPatch {
            initiative_id: Some(None),
            ..tpatch(None)
        },
        2,
    )
    .await
    .unwrap();
    assert!(s.list_todo_tags().await.unwrap().is_empty());
    s.create_todo_tagged(&todo("other", Some("i"), 1), &["i1".into()])
        .await
        .unwrap();
    s.delete_todo("other").await.unwrap();
    assert!(s.list_todo_tags().await.unwrap().is_empty());
    assert!(s.get_todo("t").await.unwrap().is_some());
}
