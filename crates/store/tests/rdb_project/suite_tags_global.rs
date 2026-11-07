//! Global tags: cross-project fallbacks and rollback on invalid definitions
//! (ported from the SQL global-tag suite).
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
async fn global_tags_work_across_projects_standalone_and_unassigned_todos() {
    let s = setup().await;
    s.create_initiative(&initiative("k", Some("q"), 0, 1))
        .await
        .unwrap();
    s.write_tag(&tag("g", "global", "", "通用")).await.unwrap();
    for (id, owner) in [
        ("a", Some("i")),
        ("b", Some("j")),
        ("c", Some("k")),
        ("d", None),
    ] {
        s.create_todo_tagged(&todo(id, owner, 1), &["g".into()])
            .await
            .unwrap();
        assert_eq!(selected(s.as_ref(), id).await, ["g"]);
    }
    s.create_todo(&todo("untagged", None, 1)).await.unwrap();
    // Upsert (rename) keeps the existing selections.
    s.write_tag(&tag("g", "global", "", "新通用"))
        .await
        .unwrap();
    assert!(selected(s.as_ref(), "untagged").await.is_empty());
    assert_eq!(selected(s.as_ref(), "d").await, ["g"]);
    s.delete_tag("g").await.unwrap();
    assert!(s.list_todo_tags().await.unwrap().is_empty());
    assert_eq!(s.list_todos(None).await.unwrap().len(), 5);
}

#[tokio::test]
async fn local_overrides_fall_back_to_project_then_global_on_delete_and_detach() {
    let s = setup().await;
    s.write_tag(&tag("g", "global", "", "模块")).await.unwrap();
    s.create_todo_tagged(&todo("t", Some("i"), 1), &["g".into()])
        .await
        .unwrap();
    // A same-name project tag takes over the global selection.
    s.write_tag(&tag("p1", "project", "p", "模块"))
        .await
        .unwrap();
    assert_eq!(selected(s.as_ref(), "t").await, ["p1"]);
    // A same-name initiative tag wins again.
    s.write_tag(&tag("i1", "initiative", "i", "模块"))
        .await
        .unwrap();
    assert_eq!(selected(s.as_ref(), "t").await, ["i1"]);
    // An inherited selection is resolved to the most specific same-name tag.
    s.create_todo_tagged(&todo("other", Some("i"), 1), &["g".into()])
        .await
        .unwrap();
    assert_eq!(selected(s.as_ref(), "other").await, ["i1"]);
    // Deleting the local override falls back to the project tag...
    s.delete_tag("i1").await.unwrap();
    assert_eq!(selected(s.as_ref(), "t").await, ["p1"]);
    // ...and detaching the todo (keeping its selection) falls back to the
    // global tag because the project tag is out of scope now.
    s.patch_todo_tagged(
        "t",
        &ProjectTodoPatch {
            initiative_id: Some(None),
            ..tpatch(None)
        },
        None,
        2,
    )
    .await
    .unwrap();
    assert_eq!(selected(s.as_ref(), "t").await, ["g"]);
    // Deleting the goal drops the project tag (and the initiative link),
    // so the shadowed selection falls back to the global tag too.
    s.delete_goal("p").await.unwrap();
    assert_eq!(selected(s.as_ref(), "other").await, ["g"]);
}

#[tokio::test]
async fn moving_global_definition_remaps_only_existing_selections() {
    let s = setup().await;
    s.write_tag(&tag("g", "global", "", "模块")).await.unwrap();
    for (id, owner) in [("a", Some("i")), ("b", Some("j")), ("c", None)] {
        s.create_todo_tagged(&todo(id, owner, 1), &["g".into()])
            .await
            .unwrap();
    }
    // Shadowing in initiative j takes over only b's selection.
    s.write_tag(&tag("local", "initiative", "j", "模块"))
        .await
        .unwrap();
    // Turning the global into a project tag keeps the in-scope selections.
    s.write_tag(&tag("g", "project", "p", "模块"))
        .await
        .unwrap();
    assert_eq!(selected(s.as_ref(), "a").await, ["g"]);
    assert_eq!(selected(s.as_ref(), "b").await, ["local"]);
    assert!(selected(s.as_ref(), "c").await.is_empty());
    // Moving it back to global restores exactly the same picture.
    s.write_tag(&tag("g", "global", "", "模块")).await.unwrap();
    assert_eq!(selected(s.as_ref(), "a").await, ["g"]);
    assert_eq!(selected(s.as_ref(), "b").await, ["local"]);
    assert!(selected(s.as_ref(), "c").await.is_empty());
}

#[tokio::test]
async fn invalid_global_owner_and_duplicate_moves_roll_back_catalog_and_links() {
    let s = setup().await;
    s.write_tag(&tag("g", "global", "", "模块")).await.unwrap();
    s.create_todo_tagged(&todo("t", None, 1), &["g".into()])
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
        assert_eq!(selected(s.as_ref(), "t").await, ["g"]);
    }
    // Re-selecting an out-of-scope project tag on a standalone todo is
    // rejected without touching the todo or its links.
    assert!(s
        .patch_todo_tagged(
            "t",
            &ProjectTodoPatch {
                title: Some("不能保存".into()),
                ..tpatch(None)
            },
            Some(&["p1".into()]),
            2,
        )
        .await
        .is_err());
    assert_eq!(s.get_todo("t").await.unwrap().unwrap().title, "todo t");
    assert_eq!(selected(s.as_ref(), "t").await, ["g"]);
}
