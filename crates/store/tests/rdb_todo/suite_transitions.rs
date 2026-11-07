//! Todo-workflow transition contracts for the rdb backend, ported from the
//! SQL suite: atomic workflow/item/event commits with per-workflow event
//! cursors, stale-generation conflicts that mutate nothing, and terminal
//! transitions with upsert semantics.

use super::*;
#[tokio::test]
async fn workflow_projection_and_event_commit_are_atomic_and_versioned() {
    let store = opencoder_store::RdbStore::open_memory().await.unwrap();
    store.create_session(&parent("parent-1")).await.unwrap();

    let seq = store
        .create_todo_workflow(
            &workflow("wf-1", 0, "pending"),
            &[item("wf-1", "step-1", 1, "pending")],
            &event("wf-1", "created"),
        )
        .await
        .unwrap();
    assert_eq!(seq, 1, "first event seq");

    // A transition rewrites workflow + items and appends the event in one
    // atomic batch; the next seq follows the workflow's own cursor.
    let seq2 = store
        .commit_todo_transition(
            &workflow("wf-1", 1, "running"),
            &[item("wf-1", "step-1", 1, "running")],
            &event("wf-1", "started"),
        )
        .await
        .unwrap();
    assert_eq!(seq2, 2);

    let w = store.get_todo_workflow("wf-1").await.unwrap().unwrap();
    assert_eq!(w.status, "running");
    assert_eq!(w.generation, 1);
    assert!(store.get_todo_workflow("missing").await.unwrap().is_none());

    let items = store.list_todo_items("wf-1").await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].status, "running");
    assert_eq!(items[0].session_history, vec!["sess-step-1".to_string()]);

    assert_eq!(store.todo_events_after("wf-1", 0).await.unwrap().len(), 2);
    assert_eq!(store.last_todo_event_seq("wf-1").await.unwrap(), 2);

    // A stale generation is rejected and mutates NOTHING (no event, no
    // workflow rewrite): the batch never commits.
    let err = store
        .commit_todo_transition(
            &workflow("wf-1", 1, "failed"),
            &[item("wf-1", "step-1", 1, "failed")],
            &event("wf-1", "stale"),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("generation conflict"), "{err}");
    let w = store.get_todo_workflow("wf-1").await.unwrap().unwrap();
    assert_eq!(w.status, "running", "rejected batch left no partial write");
    assert_eq!(store.todo_events_after("wf-1", 0).await.unwrap().len(), 2);
    assert_eq!(
        store.list_todo_items("wf-1").await.unwrap()[0].status,
        "running"
    );

    // Terminal transition with reason lands and is visible in detail.
    let mut done = workflow("wf-1", 2, "done");
    done.terminal_reason = Some("finished".into());
    let seq3 = store
        .commit_todo_transition(&done, &[], &event("wf-1", "completed"))
        .await
        .unwrap();
    assert_eq!(seq3, 3);
    let detail = store
        .get_todo_workflow_detail("wf-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.status, "done");
    assert_eq!(detail.terminal_reason.as_deref(), Some("finished"));
    assert_eq!(detail.state_json["generation"], 2);
    // Items are upserted per natural key (SQL ON CONFLICT semantics): a
    // transition that omits an item leaves its last projection in place.
    let items = store.list_todo_items("wf-1").await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].status, "running");
}
