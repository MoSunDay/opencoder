//! Tag scope moves and global tags (ported from the SQL tag suites).
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

async fn selected(s: &dyn ProjectStore, todo_id: &str) -> Vec<String> {
    s.list_todo_tags()
        .await
        .unwrap()
        .into_iter()
        .filter(|link| link.todo_id == todo_id)
        .map(|link| link.tag_id)
        .collect()
}

#[tokio::test]
async fn moving_project_tag_to_initiative_preserves_id_and_removes_out_of_scope_links() {
    let s = setup().await;
    s.create_initiative(&initiative("sibling", Some("p"), 0, 1))
        .await
        .unwrap();
    s.write_tag(&tag("moving", "project", "p", "模块"))
        .await
        .unwrap();
    s.create_todo_tagged(&todo("inside", Some("i"), 1), &["moving".into()])
        .await
        .unwrap();
    let mut outside = todo("outside", Some("sibling"), 1);
    outside.board_status = "todo".into();
    s.create_todo_tagged(&outside, &["moving".into()])
        .await
        .unwrap();
    s.create_todo(&todo("unselected", Some("i"), 1))
        .await
        .unwrap();
    s.write_tag(&tag("moving", "initiative", "i", "模块"))
        .await
        .unwrap();
    assert_eq!(
        s.list_tags().await.unwrap(),
        [tag("moving", "initiative", "i", "模块")]
    );
    assert_eq!(selected(s.as_ref(), "inside").await, ["moving"]);
    assert!(selected(s.as_ref(), "outside").await.is_empty());
    assert!(selected(s.as_ref(), "unselected").await.is_empty());
    assert_eq!(
        s.get_todo("outside").await.unwrap().unwrap().board_status,
        "todo"
    );
}

#[tokio::test]
async fn moving_local_override_restores_source_project_and_overrides_target_project() {
    let s = setup().await;
    s.create_initiative(&initiative("target", Some("q"), 0, 1))
        .await
        .unwrap();
    for definition in [
        tag("parent", "project", "p", "模块"),
        tag("local", "initiative", "i", "模块"),
        tag("target-parent", "project", "q", "模块"),
    ] {
        s.write_tag(&definition).await.unwrap();
    }
    s.create_todo_tagged(&todo("source", Some("i"), 1), &["local".into()])
        .await
        .unwrap();
    s.create_todo_tagged(
        &todo("target", Some("target"), 1),
        &["target-parent".into()],
    )
    .await
    .unwrap();
    // Moving the override into the target initiative (same name): the
    // source falls back to its project tag, the target is taken over.
    s.write_tag(&tag("local", "initiative", "target", "模块"))
        .await
        .unwrap();
    assert_eq!(selected(s.as_ref(), "source").await, ["parent"]);
    assert_eq!(selected(s.as_ref(), "target").await, ["local"]);
    // Moving it out again restores the target's project tag.
    s.write_tag(&tag("local", "initiative", "j", "模块"))
        .await
        .unwrap();
    assert_eq!(selected(s.as_ref(), "target").await, ["target-parent"]);
    // A rename along with the move breaks both the scope and the name
    // fallback: the standalone selection is dropped, the target keeps its
    // project tag.
    s.create_todo_tagged(&todo("standalone", Some("j"), 1), &["local".into()])
        .await
        .unwrap();
    s.write_tag(&tag("local", "project", "q", "专项模块"))
        .await
        .unwrap();
    assert!(selected(s.as_ref(), "standalone").await.is_empty());
    assert_eq!(selected(s.as_ref(), "target").await, ["target-parent"]);
}

#[tokio::test]
async fn moving_between_projects_rejects_duplicates_and_missing_owners_without_changes() {
    let s = setup().await;
    s.write_tag(&tag("moving", "project", "p", "模块"))
        .await
        .unwrap();
    s.write_tag(&tag("duplicate", "project", "q", "模块"))
        .await
        .unwrap();
    s.create_todo_tagged(&todo("source", Some("i"), 1), &["moving".into()])
        .await
        .unwrap();
    let before_tags = s.list_tags().await.unwrap();
    let before_links = s.list_todo_tags().await.unwrap();
    assert!(s
        .write_tag(&tag("moving", "project", "q", "模块"))
        .await
        .unwrap_err()
        .is::<opencoder_store::project::tags::TagError>());
    assert!(s
        .write_tag(&tag("moving", "initiative", "gone", "模块"))
        .await
        .unwrap_err()
        .is::<opencoder_store::project::tags::TagError>());
    assert_eq!(s.list_tags().await.unwrap(), before_tags);
    assert_eq!(s.list_todo_tags().await.unwrap(), before_links);
    // A rename alongside the move succeeds and drops out-of-scope links.
    s.write_tag(&tag("moving", "project", "q", "新模块"))
        .await
        .unwrap();
    assert!(selected(s.as_ref(), "source").await.is_empty());
    assert!(s
        .list_tags()
        .await
        .unwrap()
        .contains(&tag("moving", "project", "q", "新模块")));
}

#[tokio::test]
async fn simultaneous_rename_and_move_matches_the_saved_name() {
    let s = setup().await;
    s.write_tag(&tag("parent", "project", "p", "新模块"))
        .await
        .unwrap();
    s.write_tag(&tag("moving", "initiative", "i", "旧模块"))
        .await
        .unwrap();
    s.create_todo_tagged(&todo("source", Some("i"), 1), &["moving".into()])
        .await
        .unwrap();
    // Renaming to the same name as the destination scope's project tag
    // resolves the link to that project tag.
    s.write_tag(&tag("moving", "initiative", "j", "新模块"))
        .await
        .unwrap();
    assert_eq!(selected(s.as_ref(), "source").await, ["parent"]);
}
