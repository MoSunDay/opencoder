//! `clear_node_dialogs` — bulk-clear of a node's console dialogs at the store
//! seam. Contract: sessions of TERMINAL node tasks (done | error | cancelled)
//! are deleted in one statement (FK cascades take messages and the task row
//! with them); sessions of pending/running/cancelling tasks are kept and
//! reported as `skipped`. Runs on `open_memory`.

use opencoder_store::{LibsqlStore, NodeTaskRecord, NodeTaskStatus, Store};

async fn fresh() -> LibsqlStore {
    LibsqlStore::open_memory().await.unwrap()
}

async fn register(store: &LibsqlStore, name: &str) -> String {
    store
        .register_node(name, None, None, None, 1_000)
        .await
        .unwrap()
        .id
}

async fn dispatch(store: &LibsqlStore, id: &str, node: &str, created_at: i64) -> NodeTaskRecord {
    store
        .dispatch_node_task(
            id,
            &format!("sess-{id}"),
            node,
            Some(format!("title {id}").as_str()),
            &format!("prompt-{id}"),
            None,
            None,
            created_at,
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn clears_terminal_keeps_non_terminal_and_reports_skipped() {
    let store = fresh().await;
    let node = register(&store, "sweep").await;
    let other = register(&store, "untouched").await;

    let running = dispatch(&store, "t-run", &node, 1_000).await;
    let pending = dispatch(&store, "t-pend", &node, 2_000).await;
    let done = dispatch(&store, "t-done", &node, 3_000).await;
    let errored = dispatch(&store, "t-err", &node, 4_000).await;
    let cancelled = dispatch(&store, "t-cancel", &node, 5_000).await;
    let foreign = dispatch(&store, "t-other", &other, 5_000).await;

    let now = 10_000;
    store
        .update_node_task_status(&running.id, NodeTaskStatus::Running, None, now)
        .await
        .unwrap();
    store
        .update_node_task_status(&done.id, NodeTaskStatus::Running, None, now)
        .await
        .unwrap();
    store
        .update_node_task_status(&done.id, NodeTaskStatus::Done, None, now)
        .await
        .unwrap();
    store
        .update_node_task_status(&errored.id, NodeTaskStatus::Running, None, now)
        .await
        .unwrap();
    store
        .update_node_task_status(&errored.id, NodeTaskStatus::Error, Some("boom"), now)
        .await
        .unwrap();
    store
        .update_node_task_status(&cancelled.id, NodeTaskStatus::Cancelled, None, now)
        .await
        .unwrap();
    store
        .update_node_task_status(&foreign.id, NodeTaskStatus::Running, None, now)
        .await
        .unwrap();
    store
        .update_node_task_status(&foreign.id, NodeTaskStatus::Done, None, now)
        .await
        .unwrap();

    let result = store.clear_node_dialogs(&node).await.unwrap();
    assert_eq!(result.removed, 3, "done + error + cancelled sessions go");
    let mut skipped = result.skipped.clone();
    skipped.sort();
    assert_eq!(
        skipped,
        [
            format!("sess-{}", pending.id),
            format!("sess-{}", running.id)
        ],
        "running + pending survive, cancelling would too"
    );

    for sid in [
        &format!("sess-{}", done.id),
        &format!("sess-{}", errored.id),
        &format!("sess-{}", cancelled.id),
    ] {
        assert!(
            store.get_session(sid).await.unwrap().is_none(),
            "{sid} swept"
        );
    }
    for sid in [
        &format!("sess-{}", running.id),
        &format!("sess-{}", pending.id),
    ] {
        assert!(
            store.get_session(sid).await.unwrap().is_some(),
            "{sid} kept"
        );
    }
    // Other nodes' dialogs are untouched.
    assert!(store
        .get_session(&format!("sess-{}", foreign.id))
        .await
        .unwrap()
        .is_some());
    assert!(store
        .get_node_task_by_session(&format!("sess-{}", foreign.id))
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn empty_node_and_unknown_node_are_noops() {
    let store = fresh().await;
    let node = register(&store, "empty").await;
    let result = store.clear_node_dialogs(&node).await.unwrap();
    assert_eq!(result.removed, 0);
    assert!(result.skipped.is_empty());

    let result = store.clear_node_dialogs("ghost").await.unwrap();
    assert_eq!(result.removed, 0);
    assert!(result.skipped.is_empty());
}

#[tokio::test]
async fn cascade_removes_session_children_and_task_row() {
    let store = fresh().await;
    let node = register(&store, "cascade").await;
    let done = dispatch(&store, "t-done", &node, 1_000).await;
    let sid = done.session_id.clone();
    store
        .update_node_task_status(&done.id, NodeTaskStatus::Running, None, 1_500)
        .await
        .unwrap();
    store
        .update_node_task_status(&done.id, NodeTaskStatus::Done, None, 2_000)
        .await
        .unwrap();
    store
        .append_message(&sid, &opencoder_core::Message::user("m1", "hello"))
        .await
        .unwrap();

    let result = store.clear_node_dialogs(&node).await.unwrap();
    assert_eq!(result.removed, 1);
    assert!(store.get_session(&sid).await.unwrap().is_none());
    assert!(store.load_messages(&sid).await.unwrap().is_empty());
    assert!(store
        .get_node_task_by_session(&sid)
        .await
        .unwrap()
        .is_none());
}
