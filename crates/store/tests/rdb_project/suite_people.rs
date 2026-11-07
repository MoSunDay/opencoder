//! People assignment validation on goals/initiatives/todos: dedup on write,
//! independent owner/follower sets, full-clear patch semantics and rollback
//! of invalid references. User deletion strips the identity from every
//! embedded people list in the same batch (the SQL `ON DELETE CASCADE`).
use super::*;
use opencoder_core::identity::Role;
use opencoder_store::project::people::ProjectPeople;
use opencoder_store::Store;

async fn setup() -> Arc<RdbStore> {
    let s = mem().await;
    for name in ["alice", "bob"] {
        s.create_user(name, "", Role::Viewer, 1).await.unwrap();
    }
    let mut g = goal("p", 0, 1);
    g.people = ProjectPeople {
        owner_user_name: Some("alice".into()),
        follower_user_names: vec!["bob".into(), "alice".into(), "bob".into()],
    };
    s.create_goal(&g).await.unwrap();
    s.create_initiative(&initiative("i", Some("p"), 0, 1))
        .await
        .unwrap();
    let mut t = todo("t", Some("i"), 1);
    t.people = ProjectPeople {
        owner_user_name: Some("bob".into()),
        follower_user_names: vec!["alice".into()],
    };
    s.create_todo(&t).await.unwrap();
    s
}

#[tokio::test]
async fn people_are_independent_and_patch_can_clear_without_changing_other_roles() {
    let s = setup().await;
    // Follower lists dedup on write and drop unknown users.
    assert_eq!(
        s.list_goals().await.unwrap()[0].people.follower_user_names,
        ["alice", "bob"]
    );
    assert_eq!(
        s.list_initiatives(None).await.unwrap()[0]
            .people
            .owner_user_name,
        None
    );
    let s: Arc<dyn ProjectStore> = s.clone();
    // Clearing the owner keeps the followers.
    s.patch_todo(
        "t",
        &serde_json::from_value::<opencoder_store::ProjectTodoPatch>(
            serde_json::json!({"owner_user_name":null}),
        )
        .unwrap(),
        2,
    )
    .await
    .unwrap();
    let t = s.get_todo("t").await.unwrap().unwrap();
    assert_eq!(t.people.owner_user_name, None);
    assert_eq!(t.people.follower_user_names, ["alice"]);
    // Clearing the followers keeps the (re-set) owner.
    s.patch_todo(
        "t",
        &serde_json::from_value::<opencoder_store::ProjectTodoPatch>(
            serde_json::json!({"follower_user_names":[]}),
        )
        .unwrap(),
        3,
    )
    .await
    .unwrap();
    assert!(s
        .get_todo("t")
        .await
        .unwrap()
        .unwrap()
        .people
        .follower_user_names
        .is_empty());
    // Setting the initiative owner leaves goal people untouched.
    s.patch_initiative(
        "i",
        &serde_json::from_value::<opencoder_store::ProjectInitiativePatch>(
            serde_json::json!({"owner_user_name":"bob","goal_id":null}),
        )
        .unwrap(),
        4,
    )
    .await
    .unwrap();
    assert_eq!(
        s.list_initiatives(None).await.unwrap()[0]
            .people
            .owner_user_name
            .as_deref(),
        Some("bob")
    );
    assert_eq!(
        s.list_goals().await.unwrap()[0]
            .people
            .owner_user_name
            .as_deref(),
        Some("alice")
    );
}

#[tokio::test]
async fn invalid_people_roll_back_object_and_tag_mutations() {
    let s = setup().await;
    let s: Arc<dyn ProjectStore> = s.clone();
    assert!(s
        .patch_goal(
            "p",
            &serde_json::from_value::<opencoder_store::ProjectGoalPatch>(
                serde_json::json!({"title":"must not persist","owner_user_name":"missing"}),
            )
            .unwrap(),
            2,
        )
        .await
        .is_err());
    assert_eq!(s.list_goals().await.unwrap()[0].title, "goal p");
    assert!(s
        .create_todo(&{
            let mut rec = todo("invalid", Some("i"), 1);
            rec.people = ProjectPeople {
                owner_user_name: None,
                follower_user_names: vec!["missing".into()],
            };
            rec
        })
        .await
        .is_err());
    assert!(s.get_todo("invalid").await.unwrap().is_none());
    // An invalid people reference vetoes the whole tagged patch.
    let patch = serde_json::from_value::<opencoder_store::ProjectTodoPatch>(
        serde_json::json!({"title":"must not persist","owner_user_name":"alice","follower_user_names":[]}),
    )
    .unwrap();
    assert!(s
        .patch_todo_tagged("t", &patch, Some(&["missing-tag".into()]), 2)
        .await
        .is_err());
    let t = s.get_todo("t").await.unwrap().unwrap();
    assert_eq!(t.title, "todo t");
    assert_eq!(t.people.owner_user_name.as_deref(), Some("bob"));
    assert_eq!(t.people.follower_user_names, ["alice"]);
    assert!(s
        .patch_todo(
            "t",
            &serde_json::from_value::<opencoder_store::ProjectTodoPatch>(
                serde_json::json!({"title":"bad","follower_user_names":["missing"]}),
            )
            .unwrap(),
            3,
        )
        .await
        .is_err());
    assert_eq!(s.get_todo("t").await.unwrap().unwrap().title, "todo t");
}

#[tokio::test]
async fn deleting_objects_keeps_other_rows_people_intact() {
    let s = setup().await;
    // Deleting the user strips it from embedded owner/follower lists; a
    // later user with the same name does not re-own the object.
    assert!(s.delete_user("alice").await.unwrap());
    s.create_user("alice", "", Role::Viewer, 2).await.unwrap();
    let iface: Arc<dyn ProjectStore> = s.clone();
    let p = &iface.list_goals().await.unwrap()[0];
    assert_eq!(p.people.owner_user_name, None);
    assert_eq!(p.people.follower_user_names, ["bob"]);
    assert_eq!(
        iface
            .get_todo("t")
            .await
            .unwrap()
            .unwrap()
            .people
            .follower_user_names,
        Vec::<String>::new()
    );

    // Deleting a goal keeps todo people untouched...
    iface.delete_goal("p").await.unwrap();
    assert_eq!(
        iface
            .get_todo("t")
            .await
            .unwrap()
            .unwrap()
            .people
            .owner_user_name
            .as_deref(),
        Some("bob")
    );
    // ...and deleting the objects removes their embedded people with them.
    iface.delete_todo("t").await.unwrap();
    iface.delete_initiative("i").await.unwrap();
    assert!(iface.get_todo("t").await.unwrap().is_none());
    assert!(iface.list_initiatives(None).await.unwrap().is_empty());
    assert!(s.find_user_by_name("bob").await.unwrap().is_some());
}
