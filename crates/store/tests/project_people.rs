use opencoder_core::identity::Role;
use opencoder_store::{LibsqlStore, ProjectStore, Store};
use serde_json::{json, Value};

async fn setup() -> LibsqlStore {
    let store = LibsqlStore::open_memory().await.unwrap();
    for name in ["alice", "bob"] {
        store
            .create_user(name, &format!("test-{name}"), Role::Viewer, 1)
            .await
            .unwrap();
    }
    store.create_goal(&serde_json::from_value(json!({"id":"p","title":"Release","status":"active","sort":0,"created_at":1,"updated_at":1,"owner_user_name":"alice","follower_user_names":["bob","alice","bob"]})).unwrap()).await.unwrap();
    store.create_initiative(&serde_json::from_value(json!({"id":"i","goal_id":"p","title":"Client","sort":0,"created_at":1,"updated_at":1})).unwrap()).await.unwrap();
    store
        .create_todo(&todo(
            "t",
            json!({"owner_user_name":"bob","follower_user_names":["alice"]}),
        ))
        .await
        .unwrap();
    store
}

fn todo(id: &str, people: Value) -> opencoder_store::ProjectTodoRecord {
    let mut body = json!({"id":id,"initiative_id":"i","title":id,"draft":"verify evidence","status":"draft","agent":"act","created_at":1,"updated_at":1});
    body.as_object_mut()
        .unwrap()
        .extend(people.as_object().unwrap().clone());
    serde_json::from_value(body).unwrap()
}

#[tokio::test]
async fn people_are_independent_and_patch_can_clear_without_changing_other_roles() {
    let s = setup().await;
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
    s.patch_todo(
        "t",
        &serde_json::from_value(json!({"owner_user_name":null})).unwrap(),
        2,
    )
    .await
    .unwrap();
    let t = s.get_todo("t").await.unwrap().unwrap();
    assert_eq!(t.people.owner_user_name, None);
    assert_eq!(t.people.follower_user_names, ["alice"]);
    s.patch_todo(
        "t",
        &serde_json::from_value(json!({"follower_user_names":[],"initiative_id":null})).unwrap(),
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
    s.patch_initiative(
        "i",
        &serde_json::from_value(json!({"owner_user_name":"bob","goal_id":null})).unwrap(),
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
    let patch =
        serde_json::from_value(json!({"title":"must not persist","owner_user_name":"missing"}))
            .unwrap();
    assert!(s.patch_goal("p", &patch, 2).await.is_err());
    assert_eq!(s.list_goals().await.unwrap()[0].title, "Release");
    assert!(s
        .create_todo(&todo("invalid", json!({"follower_user_names":["missing"]})))
        .await
        .is_err());
    assert!(s.get_todo("invalid").await.unwrap().is_none());
    let patch = serde_json::from_value(
        json!({"title":"must not persist","owner_user_name":"alice","follower_user_names":[]}),
    )
    .unwrap();
    assert!(s
        .patch_todo_tagged("t", &patch, Some(&["missing-tag".into()]), 2)
        .await
        .is_err());
    let t = s.get_todo("t").await.unwrap().unwrap();
    assert_eq!(t.title, "t");
    assert_eq!(t.people.owner_user_name.as_deref(), Some("bob"));
    assert_eq!(t.people.follower_user_names, ["alice"]);
    assert!(s
        .patch_todo(
            "t",
            &serde_json::from_value(json!({"title":"bad","follower_user_names":["missing"]}))
                .unwrap(),
            3
        )
        .await
        .is_err());
    assert_eq!(s.get_todo("t").await.unwrap().unwrap().title, "t");
}

#[tokio::test]
async fn deleting_users_or_objects_cleans_only_their_people_relations() {
    let s = setup().await;
    s.delete_user("alice").await.unwrap();
    s.create_user("alice", "replacement-digest", Role::Viewer, 2)
        .await
        .unwrap();
    let p = &s.list_goals().await.unwrap()[0];
    assert_eq!(p.people.owner_user_name, None);
    assert_eq!(p.people.follower_user_names, ["bob"]);
    assert!(s
        .get_todo("t")
        .await
        .unwrap()
        .unwrap()
        .people
        .follower_user_names
        .is_empty());
    s.delete_goal("p").await.unwrap();
    assert_eq!(
        s.get_todo("t")
            .await
            .unwrap()
            .unwrap()
            .people
            .owner_user_name
            .as_deref(),
        Some("bob")
    );
    s.delete_todo("t").await.unwrap();
    s.delete_initiative("i").await.unwrap();
    let mut rows = s
        .conn()
        .await
        .unwrap()
        .query("SELECT COUNT(*) FROM project_people", ())
        .await
        .unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        0
    );
    assert!(s.find_user_by_name("bob").await.unwrap().is_some());
}

#[tokio::test]
async fn people_migration_preserves_credentials_and_starts_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let s = LibsqlStore::open(&path).await.unwrap();
    s.create_user("alice", "fixed-test-digest", Role::Editor, 1)
        .await
        .unwrap();
    let conn = s.conn().await.unwrap();
    // Model a pre-people database without touching any real credentials.
    conn.execute("DROP TRIGGER project_people_delete_project", ())
        .await
        .unwrap();
    conn.execute("DROP TRIGGER project_people_delete_initiative", ())
        .await
        .unwrap();
    conn.execute("DROP TRIGGER project_people_delete_todo", ())
        .await
        .unwrap();
    conn.execute("DROP TABLE project_people", ()).await.unwrap();
    conn.execute("UPDATE schema_version SET version=35", ())
        .await
        .unwrap();
    drop(conn);
    drop(s);
    for _ in 0..2 {
        let s = LibsqlStore::open(&path).await.unwrap();
        let conn = s.conn().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT token_hash FROM platform_tokens WHERE user_name='alice'",
                (),
            )
            .await
            .unwrap();
        assert_eq!(
            rows.next()
                .await
                .unwrap()
                .unwrap()
                .get::<String>(0)
                .unwrap(),
            "fixed-test-digest"
        );
        let mut rows = conn
            .query("SELECT COUNT(*) FROM project_people", ())
            .await
            .unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            0
        );
    }
}
