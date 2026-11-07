//! Todo event-cursor contracts for the rdb backend, ported from the SQL
//! suite: forward/reverse per-workflow cursors that stay dense against
//! sibling noise, byte-budgeted pages with omission markers and bounded
//! payload chunk recovery.

use super::*;
#[tokio::test]
async fn reverse_history_budgets_and_skips_across_workflow_gaps() {
    let store = opencoder_store::RdbStore::open_memory().await.unwrap();
    store.create_session(&parent("parent-1")).await.unwrap();
    store
        .create_todo_workflow(
            &workflow("wf-1", 0, "running"),
            &[],
            &event("wf-1", "created"),
        )
        .await
        .unwrap();
    // A sibling workflow receives hundreds of events: wf-1's own cursor
    // must stay dense and unaffected.
    store
        .create_todo_workflow(
            &workflow("other", 0, "running"),
            &[],
            &event("other", "noise"),
        )
        .await
        .unwrap();
    let mut other = workflow("other", 0, "running");
    for generation in 1..=300 {
        other.generation = generation;
        store
            .commit_todo_transition(&other, &[], &event("other", "noise"))
            .await
            .unwrap();
    }

    // An oversized dispatch payload plus a head event on wf-1.
    let mut large = event("wf-1", "dispatch");
    large.payload = serde_json::json!({ "text": "中".repeat(30_000) });
    let dispatch = store
        .commit_todo_transition(&workflow("wf-1", 1, "running"), &[], &large)
        .await
        .unwrap();
    assert_eq!(dispatch, 2, "wf-1 cursor ignores the sibling's 301 events");
    let head = store
        .commit_todo_transition(
            &workflow("wf-1", 2, "done"),
            &[],
            &event("wf-1", "completed"),
        )
        .await
        .unwrap();
    assert_eq!(head, 3);

    // Reverse window from the head: newest first, more=true.
    let page = store
        .todo_events_before("wf-1", head + 1, 200, 2048)
        .await
        .unwrap();
    assert!(page.more);
    assert_eq!(
        page.events
            .iter()
            .map(|e| e.seq.unwrap())
            .collect::<Vec<_>>(),
        vec![head]
    );

    // The oversized dispatch is served as an omission marker under the byte
    // budget; the payload is recovered through bounded chunks.
    let page = store
        .todo_events_before("wf-1", head, 200, 2048)
        .await
        .unwrap();
    assert!(page.more);
    assert_eq!(page.events[0].seq, Some(dispatch));
    assert_eq!(page.events[0].payload["omitted"], true);
    let chunk = store
        .todo_event_payload_chunk("wf-1", dispatch, 0, 64 * 1024)
        .await
        .unwrap()
        .unwrap();
    assert!(chunk.total_bytes > 64 * 1024);
    let chunk2 = store
        .todo_event_payload_chunk("wf-1", dispatch, chunk.bytes.len() as u64, 64 * 1024)
        .await
        .unwrap()
        .unwrap();
    let mut whole = chunk.bytes.clone();
    whole.extend(chunk2.bytes.clone());
    assert_eq!(whole.len() as u64, chunk.total_bytes);

    // Below the dispatch: only the created event remains, no more pages.
    let page = store
        .todo_events_before("wf-1", dispatch, 200, 2048)
        .await
        .unwrap();
    assert!(!page.more);
    assert_eq!(
        page.events
            .iter()
            .map(|e| e.seq.unwrap())
            .collect::<Vec<_>>(),
        vec![1]
    );

    // Row cap: limit=2 from the head walks newest-first.
    let page = store
        .todo_events_before("wf-1", head + 1, 2, 200_000)
        .await
        .unwrap();
    assert!(page.more);
    assert_eq!(
        page.events
            .iter()
            .map(|e| e.seq.unwrap())
            .collect::<Vec<_>>(),
        vec![head, dispatch]
    );

    // Forward cursor parity.
    let events = store.todo_events_after("wf-1", 1).await.unwrap();
    assert_eq!(
        events.iter().map(|e| e.seq.unwrap()).collect::<Vec<_>>(),
        vec![dispatch, head]
    );
}
