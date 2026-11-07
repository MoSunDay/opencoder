//! Session-deleting seams, ported from `tests/node_dialogs_clear.rs` and
//! the `delete_sessions` case of `tests/delete_sessions_and_indexes.rs`:
//! bulk-clear of a node's TERMINAL dialogs, the skipped reporting, the
//! child cascade, bulk session deletes that ignore unknown ids, and the
//! `import_messages` bulk-backfill report.

use super::*;
use opencoder_store::{SessionFilter, SessionMeta};

fn meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some(id.into()),
        agent: Some("act".into()),
        ..Default::default()
    }
}

async fn listed(store: &RdbStore) -> Vec<String> {
    store
        .list_sessions(&SessionFilter {
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect()
}

#[tokio::test]
async fn clears_terminal_keeps_non_terminal_and_reports_skipped() {
    let store = mem().await;
    let node = register(&store, "sweep", 1_000).await.id;
    let other = register(&store, "untouched", 1_000).await.id;

    let running = dispatch_simple(&store, "t-run", &node, 1_000).await;
    let pending = dispatch_simple(&store, "t-pend", &node, 2_000).await;
    let done = dispatch_simple(&store, "t-done", &node, 3_000).await;
    let errored = dispatch_simple(&store, "t-err", &node, 4_000).await;
    let cancelled = dispatch_simple(&store, "t-cancel", &node, 5_000).await;
    let foreign = dispatch_simple(&store, "t-other", &other, 5_000).await;

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
        format!("sess-{}", done.id),
        format!("sess-{}", errored.id),
        format!("sess-{}", cancelled.id),
    ] {
        assert!(
            store.get_session(&sid).await.unwrap().is_none(),
            "{sid} swept"
        );
    }
    for sid in [
        format!("sess-{}", running.id),
        format!("sess-{}", pending.id),
    ] {
        assert!(
            store.get_session(&sid).await.unwrap().is_some(),
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
    let store = mem().await;
    let node = register(&store, "empty", 1_000).await.id;
    let result = store.clear_node_dialogs(&node).await.unwrap();
    assert_eq!(result.removed, 0);
    assert!(result.skipped.is_empty());

    let result = store.clear_node_dialogs("ghost").await.unwrap();
    assert_eq!(result.removed, 0);
    assert!(result.skipped.is_empty());
}

#[tokio::test]
async fn cascade_removes_session_children_and_task_row() {
    let store = mem().await;
    let node = register(&store, "cascade", 1_000).await.id;
    let done = dispatch_simple(&store, "t-done", &node, 1_000).await;
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

#[tokio::test]
async fn delete_sessions_cascades_and_ignores_unknown_ids() {
    let store = mem().await;
    for id in ["keep", "drop-a", "drop-b"] {
        store.create_session(&meta(id)).await.unwrap();
    }
    store
        .append_message("drop-a", &opencoder_core::Message::user("m1", "cascade me"))
        .await
        .unwrap();

    let removed = store
        .delete_sessions(&["drop-a".into(), "drop-b".into(), "ghost".into()])
        .await
        .unwrap();
    assert_eq!(removed, 2);
    assert_eq!(listed(&store).await, vec!["keep".to_string()]);
    assert!(store.load_messages("drop-a").await.unwrap().is_empty());
}

/// The bulk-history backfill report: messages land in order, the report
/// counts one session and every message, nothing is skipped.
#[tokio::test]
async fn import_messages_backfills_history_with_report() {
    let store = mem().await;
    store.create_session(&meta("imported")).await.unwrap();

    let msgs = vec![
        opencoder_core::Message::user("m1", "first"),
        opencoder_core::Message::user("m2", "second"),
    ];
    let report = store.import_messages("imported", &msgs).await.unwrap();
    assert_eq!(report.sessions, 1);
    assert_eq!(report.messages, 2);
    assert_eq!(report.skipped, 0);

    let loaded = store.load_messages("imported").await.unwrap();
    assert_eq!(loaded.len(), 2);
}
