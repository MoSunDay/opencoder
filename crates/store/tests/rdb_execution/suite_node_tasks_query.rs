//! Read-half tests for the node-task query API: `list_node_tasks_filtered`
//! (fleet-wide listing with node/status filters, FIFO order, limit) and
//! `get_node_task_by_session` (synthetic-session reverse lookup). Ported
//! from `tests/node_tasks_query.rs`.

use super::*;
use opencoder_store::SessionMeta;
use opencoder_store::TASK_TYPE_NODE;

async fn register_simple(store: &RdbStore, name: &str) -> String {
    store
        .register_node(name, None, None, None, 1_000)
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn fleet_wide_filters_and_fifo_order() {
    let store = mem().await;
    let a = register_simple(&store, "a").await;
    let b = register_simple(&store, "b").await;
    dispatch_simple(&store, "a1", &a, 1_000).await;
    let a2 = dispatch_simple(&store, "a2", &a, 2_000).await;
    dispatch_simple(&store, "b1", &b, 3_000).await;

    // Advance a2 through running -> done so every filter sees mixed states.
    store
        .update_node_task_status(&a2.id, NodeTaskStatus::Running, None, 2_100)
        .await
        .unwrap();
    store
        .update_node_task_status(&a2.id, NodeTaskStatus::Done, None, 2_200)
        .await
        .unwrap();

    // Unfiltered: every task, fleet-wide, oldest first.
    let all = store
        .list_node_tasks_filtered(None, None, 100)
        .await
        .unwrap();
    let ids: Vec<&str> = all.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["a1", "a2", "b1"]);

    // Per-node only.
    let only_a = store
        .list_node_tasks_filtered(Some(&a), None, 100)
        .await
        .unwrap();
    assert_eq!(
        only_a.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["a1", "a2"]
    );

    // Status only (fleet-wide).
    let done = store
        .list_node_tasks_filtered(None, Some(NodeTaskStatus::Done), 100)
        .await
        .unwrap();
    assert_eq!(
        done.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["a2"]
    );
    assert_eq!(done[0].status, NodeTaskStatus::Done);

    // Combined: node b + done narrows to nothing here (b1 is still pending).
    let b_done = store
        .list_node_tasks_filtered(Some(&b), Some(NodeTaskStatus::Done), 100)
        .await
        .unwrap();
    assert!(b_done.is_empty());
}

#[tokio::test]
async fn fifo_order_uses_rowid_tiebreak_for_same_ms() {
    let store = mem().await;
    let node = register_simple(&store, "fifo").await;

    // All three share one millisecond -> only the insertion ordinal breaks
    // the tie, in dispatch order.
    for id in ["t-c", "t-a", "t-b"] {
        dispatch_simple(&store, id, &node, 7_777).await;
    }
    // Older ms dispatched later still sorts first by created_at.
    dispatch_simple(&store, "t-early", &node, 1_000).await;

    let listed = store
        .list_node_tasks_filtered(Some(&node), None, 100)
        .await
        .unwrap();
    let ids: Vec<&str> = listed.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        ["t-early", "t-c", "t-a", "t-b"],
        "created_at ASC primary, insertion ordinal ASC on ties"
    );
}

#[tokio::test]
async fn limit_caps_rows_and_zero_clamps_to_one() {
    let store = mem().await;
    let node = register_simple(&store, "lim").await;
    for i in 0..3 {
        dispatch_simple(&store, &format!("l{i}"), &node, 1_000 + i).await;
    }

    let two = store
        .list_node_tasks_filtered(Some(&node), None, 2)
        .await
        .unwrap();
    assert_eq!(
        two.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["l0", "l1"],
        "limit keeps the FIFO head, not the tail"
    );

    let one = store
        .list_node_tasks_filtered(Some(&node), None, 0)
        .await
        .unwrap();
    assert_eq!(one.len(), 1, "limit 0 clamps to 1 (never an empty page)");
}

#[tokio::test]
async fn session_lookup_roundtrips_and_misses_gracefully() {
    let store = mem().await;
    let node = register_simple(&store, "rev").await;
    let task = dispatch_simple(&store, "t-rev", &node, 5_000).await;

    let hit = store
        .get_node_task_by_session(&task.session_id)
        .await
        .unwrap()
        .expect("synthetic session must resolve to its task");
    assert_eq!(hit.id, "t-rev");
    assert_eq!(hit.node_id, node);
    assert_eq!(hit.status, NodeTaskStatus::Pending);

    // Ordinary (non-node) session: legal, just no task behind it.
    store
        .create_session(&SessionMeta {
            id: "plain-session".into(),
            ..SessionMeta::default()
        })
        .await
        .unwrap();
    assert!(store
        .get_node_task_by_session("plain-session")
        .await
        .unwrap()
        .is_none());
    assert!(store
        .get_node_task_by_session("nope")
        .await
        .unwrap()
        .is_none());

    // Sanity: the synthetic session really carries the node task_type.
    let meta = store.get_session(&task.session_id).await.unwrap().unwrap();
    assert_eq!(meta.task_type.as_deref(), Some(TASK_TYPE_NODE));
}
