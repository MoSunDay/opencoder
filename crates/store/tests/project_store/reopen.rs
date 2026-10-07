use super::*;
use opencoder_store::project::{ProjectAssignment, ProjectTag};
use serde_json::{json, Value};

async fn snapshot(store: &dyn ProjectStore) -> Value {
    json!({
        "goals": store.list_goals().await.unwrap(),
        "initiatives": store.list_initiatives(None).await.unwrap(),
        "todo": store.get_todo("task").await.unwrap(),
        "tags": store.list_tags().await.unwrap(),
        "links": store.list_todo_tags().await.unwrap(),
        "assignments": store.list_todo_assignments("task").await.unwrap(),
        "runs": store.list_todo_runs("task").await.unwrap(),
    })
}

#[tokio::test]
async fn reopening_libsql_preserves_catalog_board_and_execution_results() {
    let (directory, concrete, store) = fresh().await;
    store.create_goal(&goal("project", 0, 1)).await.unwrap();
    store
        .create_initiative(&initiative("group", "project", 0, 2))
        .await
        .unwrap();
    store
        .write_tag(&ProjectTag {
            id: "tag".into(),
            scope_type: "project".into(),
            scope_id: "project".into(),
            name: "delivery".into(),
        })
        .await
        .unwrap();
    store
        .create_todo_tagged(&todo("task", Some("group"), 3), &["tag".into()])
        .await
        .unwrap();
    run(store.as_ref(), "run", "task", 4).await;
    store
        .link_todo_execution(&ProjectAssignment {
            todo_id: "task".into(),
            execution_id: "execution".into(),
            kind: "agent".into(),
            name: "saved execution".into(),
            created_at: 5,
            capability_id: Some("capability".into()),
        })
        .await
        .unwrap();
    store
        .patch_todo_run(
            "run",
            &ProjectTodoRunPatch {
                output_md: Some("verified result".into()),
                ..Default::default()
            },
            6,
        )
        .await
        .unwrap();
    store
        .reorder_todos(Some("group"), "in_progress", &["task".into()], 6)
        .await
        .unwrap();
    let expected = snapshot(store.as_ref()).await;
    assert_eq!(expected["todo"]["board_status"], "in_progress");
    assert_eq!(expected["assignments"][0]["capability_id"], "capability");
    assert_eq!(expected["runs"][0]["output_md"], "verified result");
    drop(store);
    drop(concrete);

    let reopened = LibsqlStore::open(directory.path().join("test.db"))
        .await
        .unwrap();
    assert_eq!(snapshot(&reopened).await, expected);
    assert!(reopened
        .patch_todo_tagged(
            "task",
            &ProjectTodoPatch {
                title: Some("must roll back".into()),
                ..Default::default()
            },
            Some(&["missing-tag".into()]),
            7,
        )
        .await
        .is_err());
    assert_eq!(snapshot(&reopened).await, expected);
}
