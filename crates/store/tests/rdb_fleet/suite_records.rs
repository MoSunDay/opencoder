//! Node registry, five-column execution index, definitions and admission counts.
use crate::common::{assignment, dag, index, prepare, request};
use opencoder_core::fleet::{ExecutionIndex, ExecutionKind, ExecutionStatus, NodeRegistration};
use opencoder_store::fleet::FleetStore;
use serde_json::json;

fn node(id: &str, name: &str) -> NodeRegistration {
    NodeRegistration {
        protocol_version: 10,
        id: id.into(),
        name: name.into(),
        version: "v1".into(),
        maintenance_agent_id: "maint".into(),
        kinds: vec![ExecutionKind::Dag],
    }
}

fn ids(rows: Vec<ExecutionIndex>) -> Vec<String> {
    rows.into_iter().map(|row| row.id).collect()
}

#[tokio::test]
async fn node_registry_upserts_lists_in_id_order_and_unregisters() {
    let store = FleetStore::open_memory().await.unwrap();
    store.register(&node("node-b", "B")).await.unwrap();
    store.register(&node("node-a", "A")).await.unwrap();
    let mut updated = node("node-b", "B2");
    updated.kinds = vec![ExecutionKind::Agent, ExecutionKind::Team];
    store.register(&updated).await.unwrap();

    let nodes = store.nodes().await.unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].id, "node-a");
    assert_eq!(nodes[1].id, "node-b");
    assert_eq!(nodes[1].name, "B2");
    assert_eq!(
        nodes[1].kinds,
        vec![ExecutionKind::Agent, ExecutionKind::Team]
    );

    assert!(!store.unregister("node-missing").await.unwrap());
    assert!(store.unregister("node-a").await.unwrap());
    assert!(!store.unregister("node-a").await.unwrap(), "second removal");
    let remaining = store.nodes().await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, "node-b");
}

#[tokio::test]
async fn open_maps_the_control_path_to_an_rdb_directory_and_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.db");
    {
        let store = FleetStore::open(&path).await.unwrap();
        store.register(&node("node-a", "A")).await.unwrap();
        assert!(store.request_lock("brain", "request").await.is_ok());
    }
    assert!(!path.exists(), "the legacy file path is never created");
    assert!(dir.path().join("control.db.rdb").is_dir());
    assert!(path.with_extension("locks").is_dir());

    let reopened = FleetStore::open(&path).await.unwrap();
    assert_eq!(reopened.nodes().await.unwrap().len(), 1);
    assert!(
        FleetStore::open_memory()
            .await
            .unwrap()
            .request_lock("brain", "request")
            .await
            .is_err(),
        "in-memory stores have no lock directory"
    );
}

#[tokio::test]
async fn put_index_allows_status_updates_but_freezes_ownership() {
    let store = FleetStore::open_memory().await.unwrap();
    let original = dag("dag-a", 10, ExecutionStatus::Pending);
    store.put_index(&original).await.unwrap();

    let running = ExecutionIndex {
        status: ExecutionStatus::Running,
        ..original.clone()
    };
    store.put_index(&running).await.unwrap();
    assert_eq!(store.index("dag-a").await.unwrap().unwrap(), running);

    let conflicts = [
        ExecutionIndex {
            node_id: "node-b".into(),
            ..running.clone()
        },
        ExecutionIndex {
            created_at: 11,
            ..running.clone()
        },
        ExecutionIndex {
            kind: ExecutionKind::Team,
            ..running.clone()
        },
    ];
    for conflict in conflicts {
        assert!(store.put_index(&conflict).await.is_err());
        assert_eq!(store.index("dag-a").await.unwrap().unwrap(), running);
    }
    assert!(store.index("missing").await.unwrap().is_none());
}

#[tokio::test]
async fn indexes_orders_by_created_at_desc_then_id_and_applies_filters() {
    let store = FleetStore::open_memory().await.unwrap();
    store
        .put_index(&dag("dag-b", 10, ExecutionStatus::Idle))
        .await
        .unwrap();
    store
        .put_index(&dag("dag-a", 10, ExecutionStatus::Idle))
        .await
        .unwrap();
    store
        .put_index(&dag("dag-c", 20, ExecutionStatus::Idle))
        .await
        .unwrap();
    store
        .put_index(&index(
            "agent-d",
            30,
            ExecutionKind::Agent,
            ExecutionStatus::Idle,
        ))
        .await
        .unwrap();
    let mut other_node = dag("dag-e", 40, ExecutionStatus::Idle);
    other_node.node_id = "node-b".into();
    store.put_index(&other_node).await.unwrap();

    assert_eq!(
        ids(store.indexes(None, None, 10).await.unwrap()),
        ["dag-e", "agent-d", "dag-c", "dag-a", "dag-b"]
    );
    assert_eq!(
        ids(store
            .indexes(None, Some(ExecutionKind::Dag), 10)
            .await
            .unwrap()),
        ["dag-e", "dag-c", "dag-a", "dag-b"]
    );
    assert_eq!(
        ids(store.indexes(Some("node-a"), None, 10).await.unwrap()),
        ["agent-d", "dag-c", "dag-a", "dag-b"]
    );
    assert_eq!(
        ids(store
            .indexes(Some("node-a"), Some(ExecutionKind::Dag), 10)
            .await
            .unwrap()),
        ["dag-c", "dag-a", "dag-b"]
    );
    assert_eq!(
        ids(store.indexes(None, None, 0).await.unwrap()),
        ["dag-e"],
        "limit 0 clamps to one row"
    );
    assert!(store
        .indexes(Some("node-z"), None, 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn indexes_page_walks_a_keyset_cursor_without_duplicates() {
    let store = FleetStore::open_memory().await.unwrap();
    for (id, created_at) in [("dag-a", 10), ("dag-b", 20), ("dag-c", 20), ("dag-d", 30)] {
        store
            .put_index(&dag(id, created_at, ExecutionStatus::Idle))
            .await
            .unwrap();
    }

    let first = store.indexes_page(None, None, None, 2).await.unwrap();
    assert_eq!(ids(first.executions.clone()), ["dag-d", "dag-b"]);
    let cursor = first.next_cursor.expect("a third row remains");
    assert_eq!((cursor.created_at, cursor.id.as_str()), (20, "dag-b"));

    let second = store
        .indexes_page(None, None, Some(&cursor), 2)
        .await
        .unwrap();
    assert_eq!(ids(second.executions.clone()), ["dag-c", "dag-a"]);
    assert!(second.next_cursor.is_none());

    // The (node, kind) composite stream paginates independently.
    let composite = store
        .indexes_page(Some("node-a"), Some(ExecutionKind::Dag), None, 1)
        .await
        .unwrap();
    assert_eq!(ids(composite.executions.clone()), ["dag-d"]);
    let cursor = composite.next_cursor.expect("more dag rows");
    let next = store
        .indexes_page(Some("node-a"), Some(ExecutionKind::Dag), Some(&cursor), 1)
        .await
        .unwrap();
    assert_eq!(ids(next.executions), ["dag-b"]);

    let clamped = store.indexes_page(None, None, None, 0).await.unwrap();
    assert_eq!(clamped.executions.len(), 1, "limit 0 clamps to one row");
    assert!(clamped.next_cursor.is_some());
}

#[tokio::test]
async fn delete_terminal_indexes_removes_only_matching_terminal_rows_and_cascades() {
    let store = FleetStore::open_memory().await.unwrap();
    let rows = [
        dag("dag-idle", 1, ExecutionStatus::Idle),
        dag("dag-done", 2, ExecutionStatus::Done),
        dag("dag-error", 3, ExecutionStatus::Error),
        dag("dag-cancelled", 4, ExecutionStatus::Cancelled),
        dag("dag-running", 5, ExecutionStatus::Running),
        dag("dag-pending", 6, ExecutionStatus::Pending),
    ];
    for row in &rows {
        store.put_index(row).await.unwrap();
    }
    let mut other_node = dag("dag-other", 7, ExecutionStatus::Done);
    other_node.node_id = "node-b".into();
    store.put_index(&other_node).await.unwrap();
    store
        .put_index(&index(
            "team-done",
            8,
            ExecutionKind::Team,
            ExecutionStatus::Done,
        ))
        .await
        .unwrap();

    let target = store.index("dag-done").await.unwrap().unwrap();
    let dispatch = assignment(
        target,
        request("dag-done", ExecutionKind::Dag, json!({"prompt": "once"})),
        Some(json!({"version": 1})),
    );
    prepare(&store, &dispatch).await;
    assert!(store.assignment("dag-done").await.unwrap().is_some());

    let candidates: Vec<String> = [
        "dag-idle",
        "dag-done",
        "dag-error",
        "dag-cancelled",
        "dag-running",
        "dag-pending",
        "dag-other",
        "team-done",
        "missing",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let removed = store
        .delete_terminal_indexes("node-a", ExecutionKind::Dag, &candidates)
        .await
        .unwrap();
    assert_eq!(removed, 4, "idle, done, error and cancelled rows");

    for gone in ["dag-idle", "dag-done", "dag-error", "dag-cancelled"] {
        assert!(store.index(gone).await.unwrap().is_none(), "{gone}");
    }
    assert_eq!(
        store.index("dag-running").await.unwrap().unwrap().status,
        ExecutionStatus::Running,
        "live rows are never deleted"
    );
    assert_eq!(
        store.index("dag-pending").await.unwrap().unwrap().status,
        ExecutionStatus::Pending
    );
    assert!(store.index("dag-other").await.unwrap().is_some());
    assert!(store.index("team-done").await.unwrap().is_some());
    assert!(
        store.assignment("dag-done").await.unwrap().is_none(),
        "frozen assignment is deleted with the index row"
    );
    assert!(store
        .receipt("execution", "dag-done")
        .await
        .unwrap()
        .is_none());
    let remaining = ids(store.indexes(None, None, 100).await.unwrap());
    assert_eq!(
        remaining,
        ["team-done", "dag-other", "dag-pending", "dag-running"]
    );

    assert_eq!(
        store
            .delete_terminal_indexes("node-a", ExecutionKind::Dag, &candidates)
            .await
            .unwrap(),
        0,
        "already-deleted rows are not counted again"
    );
    assert_eq!(
        store
            .delete_terminal_indexes("node-a", ExecutionKind::Dag, &[])
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn definitions_round_trip_upsert_list_and_delete() {
    let store = FleetStore::open_memory().await.unwrap();
    assert!(store.definition("skill", "a").await.unwrap().is_none());
    assert!(store.definitions("skill").await.unwrap().is_empty());

    store
        .put_definition("skill", "b", &json!({"name": "b", "version": 1}))
        .await
        .unwrap();
    store
        .put_definition("skill", "a", &json!({"name": "a", "version": 1}))
        .await
        .unwrap();
    store
        .put_definition("skill", "b", &json!({"name": "b", "version": 2}))
        .await
        .unwrap();

    let definitions = store.definitions("skill").await.unwrap();
    assert_eq!(definitions.len(), 2);
    assert_eq!(definitions[0]["name"], "a");
    assert_eq!(definitions[1]["version"], 2, "upsert replaces the row");
    assert_eq!(
        store.definition("skill", "b").await.unwrap().unwrap(),
        json!({"name": "b", "version": 2})
    );
    assert!(
        store.definition("other", "b").await.unwrap().is_none(),
        "kinds are isolated"
    );

    store.delete_definition("skill", "a").await.unwrap();
    store.delete_definition("skill", "missing").await.unwrap();
    assert_eq!(
        store.definitions("skill").await.unwrap(),
        vec![json!({"name": "b", "version": 2})]
    );
}

#[tokio::test]
async fn active_execution_count_counts_only_live_statuses() {
    let store = FleetStore::open_memory().await.unwrap();
    let rows = [
        ("agent-live-1", ExecutionStatus::Pending),
        ("agent-live-2", ExecutionStatus::Running),
        ("agent-live-3", ExecutionStatus::Idle),
        ("agent-live-4", ExecutionStatus::Cancelling),
        ("agent-idle", ExecutionStatus::Interrupted),
        ("agent-done", ExecutionStatus::Done),
        ("agent-error", ExecutionStatus::Error),
        ("agent-cancelled", ExecutionStatus::Cancelled),
    ];
    for (offset, (id, status)) in rows.into_iter().enumerate() {
        store
            .put_index(&index(id, offset as i64, ExecutionKind::Agent, status))
            .await
            .unwrap();
    }
    assert_eq!(store.active_execution_count().await.unwrap(), 4);

    let finished = index(
        "agent-live-2",
        1,
        ExecutionKind::Agent,
        ExecutionStatus::Done,
    );
    store.put_index(&finished).await.unwrap();
    assert_eq!(store.active_execution_count().await.unwrap(), 3);
}
