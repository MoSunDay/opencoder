use super::*;

async fn selected(s: &TursoStore, id: &str) -> Vec<String> {
    s.list_todo_tags()
        .await
        .unwrap()
        .into_iter()
        .filter(|link| link.todo_id == id)
        .map(|link| link.tag_id)
        .collect()
}

#[tokio::test]
async fn global_tags_work_across_projects_standalone_and_unassigned_todos() {
    let s = setup().await;
    s.create_initiative(&initiative("k", Some("q")))
        .await
        .unwrap();
    s.write_tag(&tag("g", "global", "", "通用")).await.unwrap();
    for (id, owner) in [
        ("a", Some("i")),
        ("b", Some("j")),
        ("c", Some("k")),
        ("d", None),
    ] {
        s.create_todo_tagged(&todo(id, owner), &["g".into()])
            .await
            .unwrap();
        assert_eq!(selected(&s, id).await, ["g"]);
    }
    s.create_todo(&todo("untagged", None)).await.unwrap();
    s.write_tag(&tag("g", "global", "", "新通用"))
        .await
        .unwrap();
    assert!(selected(&s, "untagged").await.is_empty());
    assert_eq!(selected(&s, "d").await, ["g"]);
    s.delete_tag("g").await.unwrap();
    assert!(s.list_todo_tags().await.unwrap().is_empty());
    assert_eq!(s.list_todos(None).await.unwrap().len(), 5);
}

#[tokio::test]
async fn local_overrides_fall_back_to_project_then_global_on_delete_and_detach() {
    let s = setup().await;
    s.write_tag(&tag("g", "global", "", "模块")).await.unwrap();
    s.create_todo_tagged(&todo("t", Some("i")), &["g".into()])
        .await
        .unwrap();
    s.write_tag(&tag("p1", "project", "p", "模块"))
        .await
        .unwrap();
    assert_eq!(selected(&s, "t").await, ["p1"]);
    s.write_tag(&tag("i1", "initiative", "i", "模块"))
        .await
        .unwrap();
    assert_eq!(selected(&s, "t").await, ["i1"]);
    // A valid inherited selection is resolved to the most specific same-name tag.
    s.create_todo_tagged(&todo("other", Some("i")), &["g".into()])
        .await
        .unwrap();
    assert_eq!(selected(&s, "other").await, ["i1"]);
    s.delete_tag("i1").await.unwrap();
    assert_eq!(selected(&s, "t").await, ["p1"]);
    s.patch_todo_tagged(
        "t",
        &ProjectTodoPatch {
            initiative_id: Some(None),
            ..Default::default()
        },
        None,
        2,
    )
    .await
    .unwrap();
    assert_eq!(selected(&s, "t").await, ["g"]);
    s.delete_goal("p").await.unwrap();
    assert_eq!(selected(&s, "other").await, ["g"]);
}

#[tokio::test]
async fn moving_global_definition_remaps_only_existing_selections() {
    let s = setup().await;
    s.write_tag(&tag("g", "global", "", "模块")).await.unwrap();
    for (id, owner) in [("a", Some("i")), ("b", Some("j")), ("c", None)] {
        s.create_todo_tagged(&todo(id, owner), &["g".into()])
            .await
            .unwrap();
    }
    s.write_tag(&tag("local", "initiative", "j", "模块"))
        .await
        .unwrap();
    s.write_tag(&tag("g", "project", "p", "模块"))
        .await
        .unwrap();
    assert_eq!(selected(&s, "a").await, ["g"]);
    assert_eq!(selected(&s, "b").await, ["local"]);
    assert!(selected(&s, "c").await.is_empty());
    s.write_tag(&tag("g", "global", "", "模块")).await.unwrap();
    assert_eq!(selected(&s, "a").await, ["g"]);
    assert_eq!(selected(&s, "b").await, ["local"]);
    assert!(selected(&s, "c").await.is_empty());
}

#[tokio::test]
async fn invalid_global_owner_and_duplicate_moves_roll_back_catalog_and_links() {
    let s = setup().await;
    s.write_tag(&tag("g", "global", "", "模块")).await.unwrap();
    s.create_todo_tagged(&todo("t", None), &["g".into()])
        .await
        .unwrap();
    s.write_tag(&tag("p1", "project", "p", "模块"))
        .await
        .unwrap();
    let before = s.list_tags().await.unwrap();
    for invalid in [
        tag("g", "global", "p", "改名"),
        tag("duplicate", "global", "", "模块"),
        tag("p1", "global", "", "模块"),
    ] {
        assert!(s.write_tag(&invalid).await.is_err());
        assert_eq!(s.list_tags().await.unwrap(), before);
        assert_eq!(selected(&s, "t").await, ["g"]);
    }
    assert!(s
        .patch_todo_tagged(
            "t",
            &ProjectTodoPatch {
                title: Some("不能保存".into()),
                ..Default::default()
            },
            Some(&["p1".into()]),
            2
        )
        .await
        .is_err());
    assert_eq!(s.get_todo("t").await.unwrap().unwrap().title, "t");
    assert_eq!(selected(&s, "t").await, ["g"]);
}
