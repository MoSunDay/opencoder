//! Todo listing contracts for the rdb backend, ported from the SQL suite:
//! newest-first workflow listing limits, keyset item pagination, bounded
//! field-chunk reads and session cascade deletes.

use super::*;
#[tokio::test]
async fn listings_item_pagination_and_field_chunk_windows() {
    let store = opencoder_store::RdbStore::open_memory().await.unwrap();
    store.create_session(&parent("parent-1")).await.unwrap();
    store.create_session(&parent("parent-2")).await.unwrap();

    // Two workflows: listing truncates to the limit, newest first.
    store
        .create_todo_workflow(
            &workflow("wf-old", 0, "done"),
            &[
                item("wf-old", "a", 1, "done"),
                item("wf-old", "b", 2, "done"),
            ],
            &event("wf-old", "created"),
        )
        .await
        .unwrap();
    store
        .create_todo_workflow(
            &workflow("wf-new", 0, "pending"),
            &[
                item("wf-new", "a", 2, "pending"),
                item("wf-new", "b", 1, "pending"),
                item("wf-new", "c", 3, "pending"),
            ],
            &event("wf-new", "created"),
        )
        .await
        .unwrap();
    let listed: Vec<String> = store
        .list_todo_workflows(10)
        .await
        .unwrap()
        .into_iter()
        .map(|s: TodoWorkflowSummary| s.id)
        .collect();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0], "wf-new", "updated_at DESC");
    assert_eq!(store.list_todo_workflows(1).await.unwrap().len(), 1);

    // Items list in ordinal order regardless of write order.
    let ids: Vec<String> = store
        .list_todo_items("wf-new")
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.todo_id)
        .collect();
    assert_eq!(ids, vec!["b", "a", "c"]);

    // Keyset pagination walks ordinals 1 -> 2 -> 3.
    let page1 = store.list_todo_items_page("wf-new", None, 2).await.unwrap();
    assert_eq!(page1.items.len(), 2);
    assert_eq!(page1.items[0].todo_id, "b");
    assert_eq!(page1.next_ordinal, Some(2));
    let page2 = store
        .list_todo_items_page("wf-new", page1.next_ordinal, 2)
        .await
        .unwrap();
    assert_eq!(page2.items.len(), 1);
    assert_eq!(page2.items[0].todo_id, "c");
    assert_eq!(page2.next_ordinal, None);

    // Field chunks tile stored field bytes exactly.
    let mut rich = item("wf-new", "a", 2, "running");
    rich.result_json = Some(serde_json::json!({ "output": "x".repeat(50) }));
    rich.last_error = Some("e".repeat(100));
    store
        .commit_todo_transition(
            &workflow("wf-new", 1, "running"),
            &[rich],
            &event("wf-new", "step"),
        )
        .await
        .unwrap();
    let full = store
        .todo_item_field_chunk("wf-new", "a", "result_json", 0, 64 * 1024)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(full.bytes.len() as u64, full.total_bytes);
    let head = store
        .todo_item_field_chunk("wf-new", "a", "result_json", 0, 8)
        .await
        .unwrap()
        .unwrap();
    let tail = store
        .todo_item_field_chunk("wf-new", "a", "result_json", 8, 64 * 1024)
        .await
        .unwrap()
        .unwrap();
    let mut joined = head.bytes.clone();
    joined.extend(tail.bytes.clone());
    assert_eq!(joined, full.bytes);
    assert!(store
        .todo_item_field_chunk("wf-new", "a", "last_error", 0, 64 * 1024)
        .await
        .unwrap()
        .is_some());
    assert!(store
        .todo_item_field_chunk("wf-new", "missing", "result_json", 0, 8)
        .await
        .unwrap()
        .is_none());
    assert!(store
        .todo_item_field_chunk("wf-new", "b", "result_json", 0, 8)
        .await
        .unwrap()
        .is_none());

    // Workflow field chunks cover spec/state JSON.
    let spec = store
        .todo_workflow_field_chunk("wf-new", "spec_json", 0, 64 * 1024)
        .await
        .unwrap()
        .unwrap();
    let spec_value: serde_json::Value = serde_json::from_slice(&spec.bytes).unwrap();
    assert_eq!(spec_value["id"], "wf-new");
    assert!(store
        .todo_workflow_field_chunk("wf-new", "bogus", 0, 8)
        .await
        .is_err());

    // Deleting the owning session cascades its workflows (and only those).
    let mut other_owner = workflow("wf-old", 1, "done");
    other_owner.parent_session_id = "parent-2".into();
    store
        .commit_todo_transition(&other_owner, &[], &event("wf-old", "closed"))
        .await
        .unwrap();
    store.delete_session("parent-1").await.unwrap();
    assert!(store.get_todo_workflow("wf-new").await.unwrap().is_none());
    assert!(store.list_todo_items("wf-new").await.unwrap().is_empty());
    assert!(store
        .todo_events_after("wf-new", 0)
        .await
        .unwrap()
        .is_empty());
    let surviving: Vec<String> = store
        .list_todo_workflows(10)
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(
        surviving,
        vec!["wf-old"],
        "only the deleted session's workflows cascade"
    );
}
