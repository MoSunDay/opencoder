use opencoder_store::{project::InitiativeOrderConflict, LibsqlStore, ProjectStore};
use serde_json::json;

async fn seed(store: &LibsqlStore) {
    for id in ["p", "other"] {
        store
            .create_goal(
                &serde_json::from_value(json!({
                    "id":id,"title":id,"status":"active","sort":0,"created_at":1,"updated_at":1
                }))
                .unwrap(),
            )
            .await
            .unwrap();
    }
    for (id, goal) in [("a", "p"), ("b", "p"), ("c", "p"), ("outside", "other")] {
        store
            .create_initiative(
                &serde_json::from_value(json!({
                    "id":id,"goal_id":goal,"title":id,"sort":0,
                    "created_at":1,"updated_at":1,"current_summary":"saved","summary_updated_at":1
                }))
                .unwrap(),
            )
            .await
            .unwrap();
    }
}
fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|id| id.to_string()).collect()
}
async fn order(store: &LibsqlStore) -> Vec<String> {
    store
        .list_initiatives(Some("p"))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.id)
        .collect()
}

#[tokio::test]
async fn initiative_order_persists_and_rejects_stale_incomplete_or_foreign_lists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("order.db");
    let store = LibsqlStore::open(&path).await.unwrap();
    seed(&store).await;
    let original = ids(&["a", "b", "c"]);
    let next = ids(&["c", "a", "b"]);
    assert_eq!(order(&store).await, original);
    assert!(store
        .reorder_initiatives("p", &original, &next, 20)
        .await
        .unwrap());
    assert_eq!(order(&store).await, next);
    for (expected, invalid) in [
        (original.clone(), original),                // stale order
        (next.clone(), ids(&["a", "b"])),            // omitted initiative
        (next.clone(), ids(&["a", "a", "b"])),       // duplicate
        (next.clone(), ids(&["a", "b", "outside"])), // different project
    ] {
        let error = store
            .reorder_initiatives("p", &expected, &invalid, 30)
            .await
            .unwrap_err();
        assert!(error.is::<InitiativeOrderConflict>());
        assert_eq!(order(&store).await, next);
    }
    assert!(!store
        .reorder_initiatives("missing", &[], &[], 30)
        .await
        .unwrap());
    let outside = store
        .list_initiatives(Some("other"))
        .await
        .unwrap()
        .remove(0);
    assert_eq!(outside.sort, 0);
    assert_eq!(outside.updated_at, 1);
    for row in store.list_initiatives(Some("p")).await.unwrap() {
        assert_eq!(row.summary.current_summary, "saved");
        assert_eq!(row.summary.summary_updated_at, Some(1));
        assert_eq!(
            store
                .summary_versions(
                    opencoder_store::project::summary::SummaryScope::Initiative,
                    &row.id
                )
                .await
                .unwrap()
                .unwrap()
                .len(),
            1
        );
    }
    drop(store);
    let store = LibsqlStore::open(&path).await.unwrap();
    assert_eq!(order(&store).await, next);
}

#[tokio::test]
async fn initiative_order_write_failure_rolls_back_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("atomic.db");
    let store = LibsqlStore::open(&path).await.unwrap();
    seed(&store).await;
    let db = libsql::Builder::new_local(&path).build().await.unwrap();
    db.connect().unwrap().execute("CREATE TRIGGER fail_order BEFORE UPDATE OF sort_key ON project_initiatives WHEN NEW.id = 'b' BEGIN SELECT RAISE(ABORT, 'order failed'); END", ()).await.unwrap();
    assert!(store
        .reorder_initiatives("p", &ids(&["a", "b", "c"]), &ids(&["c", "b", "a"]), 20)
        .await
        .is_err());
    assert_eq!(order(&store).await, ids(&["a", "b", "c"]));
    assert!(store
        .list_initiatives(Some("p"))
        .await
        .unwrap()
        .iter()
        .all(|row| row.sort == 0 && row.updated_at == 1));
}
