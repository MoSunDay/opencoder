use super::*;
use crate::fleet::handoff::RuntimeRecord;
use libsql::Builder;

async fn register(store: &FleetStore, id: &str) {
    store
        .register_runtime(&RuntimeRecord {
            id: id.into(),
            release_id: format!("release-{id}"),
            config: serde_json::json!({}),
            mode: "staged".into(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn large_inventory_preserves_owners_across_activation_and_duplicate_ids() {
    let store = FleetStore::open_memory().await.unwrap();
    register(&store, "old").await;
    register(&store, "new").await;
    store.activate_runtime("old").await.unwrap();
    let names: Vec<_> = (0..1500).map(|n| format!("execution-{n}")).collect();
    let mut ids: Vec<_> = names.iter().map(String::as_str).collect();
    ids.push(ids[0]);
    store.assign_runtime_inventory("old", &ids).await.unwrap();
    store.activate_runtime("new").await.unwrap();
    store.assign_runtime_inventory("old", &ids).await.unwrap();
    assert_eq!(store.assign_runtime("fresh", None).await.unwrap(), "new");
    let mut rows = store
        .conn
        .query(
            "SELECT count(*) FROM runtime_owners WHERE runtime_id='old'",
            (),
        )
        .await
        .unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        1500
    );
    assert_eq!(
        store
            .owner("execution-1499")
            .await
            .unwrap()
            .unwrap()
            .runtime_id,
        "old"
    );
}

#[tokio::test]
async fn inventory_conflict_does_not_assign_earlier_unseen_ids() {
    let store = FleetStore::open_memory().await.unwrap();
    register(&store, "old").await;
    register(&store, "new").await;
    store.assign_runtime("owned", Some("old")).await.unwrap();
    assert!(store
        .assign_runtime_inventory("new", &["unseen", "owned"])
        .await
        .is_err());
    assert!(store.owner("unseen").await.unwrap().is_none());
    assert_eq!(
        store.owner("owned").await.unwrap().unwrap().runtime_id,
        "old"
    );
    assert!(store
        .assign_runtime_inventory("missing", &["unseen"])
        .await
        .is_err());
    assert!(store.owner("unseen").await.unwrap().is_none());
    store
        .assign_runtime_inventory("new", &["unseen"])
        .await
        .unwrap();
}

#[tokio::test]
async fn known_inventory_is_read_only_while_another_connection_holds_writer_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let store = FleetStore::open(&path).await.unwrap();
    register(&store, "old").await;
    register(&store, "new").await;
    store
        .assign_runtime_inventory("old", &["one", "two"])
        .await
        .unwrap();
    store
        .conn
        .execute_batch("PRAGMA busy_timeout=0")
        .await
        .unwrap();
    let database = Builder::new_local(&path).build().await.unwrap();
    let connection = database.connect().unwrap();
    let _writer = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .unwrap();
    store
        .assign_runtime_inventory("old", &["one", "two", "one"])
        .await
        .unwrap();
    let error = store
        .assign_runtime_inventory("new", &["one"])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ownership conflict"));
}

#[tokio::test]
async fn racing_inventories_never_reassign_a_shared_execution() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let first = FleetStore::open(&path).await.unwrap();
    let second = FleetStore::open(&path).await.unwrap();
    register(&first, "old").await;
    register(&first, "new").await;
    let (left, right) = tokio::join!(
        first.assign_runtime_inventory("old", &["shared", "left"]),
        second.assign_runtime_inventory("new", &["shared", "right"])
    );
    assert_ne!(left.is_ok(), right.is_ok());
    let winner = if left.is_ok() { "old" } else { "new" };
    let absent = if left.is_ok() { "right" } else { "left" };
    assert_eq!(
        first.owner("shared").await.unwrap().unwrap().runtime_id,
        winner
    );
    assert!(first.owner(absent).await.unwrap().is_none());
}
