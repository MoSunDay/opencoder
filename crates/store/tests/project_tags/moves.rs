use super::*;
use opencoder_store::project::tags::TagError;

async fn selected(store: &LibsqlStore, todo_id: &str) -> Vec<String> {
    store
        .list_todo_tags()
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
    s.create_initiative(&initiative("sibling", Some("p")))
        .await
        .unwrap();
    s.write_tag(&tag("moving", "project", "p", "模块"))
        .await
        .unwrap();
    s.create_todo_tagged(&todo("inside", Some("i")), &["moving".into()])
        .await
        .unwrap();
    s.create_todo_tagged(&todo("outside", Some("sibling")), &["moving".into()])
        .await
        .unwrap();
    s.create_todo(&todo("unselected", Some("i"))).await.unwrap();
    s.write_tag(&tag("moving", "initiative", "i", "模块"))
        .await
        .unwrap();
    assert_eq!(
        s.list_tags().await.unwrap(),
        [tag("moving", "initiative", "i", "模块")]
    );
    assert_eq!(selected(&s, "inside").await, ["moving"]);
    assert!(selected(&s, "outside").await.is_empty());
    assert!(selected(&s, "unselected").await.is_empty());
    assert_eq!(
        s.get_todo("outside").await.unwrap().unwrap().board_status,
        "todo"
    );
}

#[tokio::test]
async fn moving_local_override_restores_source_project_and_overrides_target_project() {
    let s = setup().await;
    s.create_initiative(&initiative("target", Some("q")))
        .await
        .unwrap();
    for definition in [
        tag("parent", "project", "p", "模块"),
        tag("local", "initiative", "i", "模块"),
        tag("target-parent", "project", "q", "模块"),
    ] {
        s.write_tag(&definition).await.unwrap();
    }
    s.create_todo_tagged(&todo("source", Some("i")), &["local".into()])
        .await
        .unwrap();
    s.create_todo_tagged(&todo("target", Some("target")), &["target-parent".into()])
        .await
        .unwrap();
    s.write_tag(&tag("local", "initiative", "target", "模块"))
        .await
        .unwrap();
    assert_eq!(selected(&s, "source").await, ["parent"]);
    assert_eq!(selected(&s, "target").await, ["local"]);
    s.write_tag(&tag("local", "initiative", "j", "模块"))
        .await
        .unwrap();
    assert_eq!(selected(&s, "target").await, ["target-parent"]);
    s.create_todo_tagged(&todo("standalone", Some("j")), &["local".into()])
        .await
        .unwrap();
    s.write_tag(&tag("local", "project", "q", "专项模块"))
        .await
        .unwrap();
    assert!(selected(&s, "standalone").await.is_empty());
    assert_eq!(selected(&s, "target").await, ["target-parent"]);
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
    s.create_todo_tagged(&todo("source", Some("i")), &["moving".into()])
        .await
        .unwrap();
    let before_tags = s.list_tags().await.unwrap();
    let before_links = s.list_todo_tags().await.unwrap();
    assert!(s
        .write_tag(&tag("moving", "project", "q", "模块"))
        .await
        .unwrap_err()
        .is::<TagError>());
    assert!(s
        .write_tag(&tag("moving", "initiative", "gone", "模块"))
        .await
        .unwrap_err()
        .is::<TagError>());
    assert_eq!(s.list_tags().await.unwrap(), before_tags);
    assert_eq!(s.list_todo_tags().await.unwrap(), before_links);
    s.write_tag(&tag("moving", "project", "q", "新模块"))
        .await
        .unwrap();
    assert!(selected(&s, "source").await.is_empty());
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
    s.create_todo_tagged(&todo("source", Some("i")), &["moving".into()])
        .await
        .unwrap();
    s.write_tag(&tag("moving", "initiative", "j", "新模块"))
        .await
        .unwrap();
    assert_eq!(selected(&s, "source").await, ["parent"]);
}
