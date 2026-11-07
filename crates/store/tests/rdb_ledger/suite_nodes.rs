//! Worker-node registry: name-keyed upsert registration, listing order,
//! deletion and heartbeat liveness convergence.

use super::*;

const NODE_A: &str = "node-alpha";
const NODE_B: &str = "node-beta";

#[tokio::test]
async fn register_list_get_delete_roundtrip() {
    let store = mem().await;

    let first = register(&store, NODE_A, 1000).await;
    assert_eq!(first.name, NODE_A);
    assert_eq!(first.version.as_deref(), Some("v1"));
    assert_eq!(first.workdir.as_deref(), Some("/tmp/wd"));
    assert_eq!(first.first_seen, 1000);
    assert_eq!(first.last_seen_at, 1000);
    assert_eq!(first.last_status, "online");
    assert_eq!(first.last_task_id, None);

    // Same-name re-registration: the original id survives (dispatched tasks
    // keep their FK), timestamps refresh, and the new metadata lands —
    // including a `None` overwriting a previous value, like the SQL
    // ON CONFLICT clause.
    let again = store
        .register_node(NODE_A, None, Some("/new/wd"), Some("10.0.0.1:1"), 9000)
        .await
        .unwrap();
    assert_eq!(again.id, first.id, "re-register must reuse the node id");
    assert_eq!(again.first_seen, 1000, "first_seen is birth metadata");
    assert_eq!(again.last_seen_at, 9000, "last_seen must refresh");
    assert_eq!(again.version, None);
    assert_eq!(again.workdir.as_deref(), Some("/new/wd"));
    assert_eq!(again.last_addr.as_deref(), Some("10.0.0.1:1"));
    assert_eq!(again.last_status, "online");

    let listed = store.list_nodes().await.unwrap();
    assert_eq!(listed.len(), 1);

    // A distinct name is a distinct node row; listing is oldest-first.
    let b = register(&store, NODE_B, 1001).await;
    let listed = store.list_nodes().await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].name, NODE_A);
    assert_eq!(listed[1].name, NODE_B);

    // Deletion targets the exact row; only that node disappears …
    store.delete_node(&b.id).await.unwrap();
    assert!(store.get_node(&b.id).await.unwrap().is_none());
    assert!(store.get_node(&first.id).await.unwrap().is_some());
    assert_eq!(store.list_nodes().await.unwrap().len(), 1);

    // … and the freed name can register fresh (index cleanup left no trace).
    let reborn = register(&store, NODE_B, 2000).await;
    assert_ne!(reborn.id, b.id, "a freed name gets a fresh id");
    assert_eq!(reborn.first_seen, 2000);
    assert_eq!(store.list_nodes().await.unwrap().len(), 2);

    assert!(store.get_node("missing").await.unwrap().is_none());
}

#[tokio::test]
async fn heartbeat_touches_liveness_and_converges_status() {
    let store = mem().await;
    // A fresh registration reports `online`; the first heartbeat collapses
    // the non-busy status toward `idle` while refreshing the timestamp.
    let node = register(&store, NODE_A, 1000).await;
    assert_eq!(
        store.heartbeat_node(&node.id, 5000).await.unwrap(),
        Vec::<String>::new(),
        "no dispatch queue yet, so no cancel commands"
    );
    let touched = store.get_node(&node.id).await.unwrap().unwrap();
    assert_eq!(touched.last_seen_at, 5000);
    assert_eq!(
        touched.last_status, "idle",
        "non-busy collapses toward idle"
    );

    // Re-registering revives `online`; the next heartbeat converges again.
    register(&store, NODE_A, 6000).await;
    assert_eq!(
        store.get_node(&node.id).await.unwrap().unwrap().last_status,
        "online"
    );
    store.heartbeat_node(&node.id, 6500).await.unwrap();
    assert_eq!(
        store.get_node(&node.id).await.unwrap().unwrap().last_status,
        "idle"
    );

    // An unknown node id hard-errors so infrastructure mistakes surface.
    assert!(store.heartbeat_node("missing", 5400).await.is_err());
}
