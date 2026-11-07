//! Event-cursor and subagent-task contracts for the rdb backend, ported
//! from the SQL suites: batch append with per-session seqs, SSE replay
//! cursors, bounded event pages with omission markers, and the subagent
//! task lifecycle with cascade cleanup.

use super::*;
use opencoder_store::{SubagentStatus, SubagentTaskRecord};

#[tokio::test]
async fn event_batch_append_and_replay_cursor_semantics() {
    let store = mem().await;
    store.create_session(&meta_for("s1", 1)).await.unwrap();
    store.create_session(&meta_for("s2", 1)).await.unwrap();

    let batch: Vec<_> = (0..5)
        .map(|i| {
            event(
                "s1",
                if i % 2 == 0 { "text_delta" } else { "step" },
                &format!(r#"{{"i":{i}}}"#),
            )
        })
        .collect();
    let seqs = store.append_events(&batch).await.unwrap();
    assert_eq!(
        seqs,
        (1..=5).collect::<Vec<i64>>(),
        "contiguous per-session seqs"
    );
    assert_eq!(store.last_event_seq("s1").await.unwrap(), 5);

    // Replay from a mid-cursor returns exactly the tail, seqs ascending.
    let tail = store.events_after("s1", 2).await.unwrap();
    assert_eq!(
        tail.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![Some(3), Some(4), Some(5)]
    );
    assert_eq!(tail[0].kind, opencoder_store::EventKind::TextDelta);
    assert_eq!(tail[0].payload["i"], 2);

    // Full replay from 0; empty replay past the head.
    assert_eq!(store.events_after("s1", 0).await.unwrap().len(), 5);
    assert!(store.events_after("s1", 5).await.unwrap().is_empty());

    // Sessions are isolated cursors.
    let seqs2 = store
        .append_events(&[event("s2", "done", "{}")])
        .await
        .unwrap();
    assert_eq!(seqs2, vec![1], "per-session seq restarts at 1");
    assert_eq!(store.last_event_seq("s2").await.unwrap(), 1);
    assert_eq!(store.events_after("s2", 0).await.unwrap().len(), 1);
    assert_eq!(store.last_event_seq("nope").await.unwrap(), 0);

    // Mixed-session batches are rejected atomically (nothing recorded).
    let mixed = vec![event("s1", "step", "{}"), event("s2", "step", "{}")];
    assert!(store.append_events(&mixed).await.is_err());
    assert_eq!(store.last_event_seq("s1").await.unwrap(), 5);
    assert_eq!(store.last_event_seq("s2").await.unwrap(), 1);
}

#[tokio::test]
async fn event_pages_respect_row_limit_and_byte_budget() {
    let store = mem().await;
    store.create_session(&meta_for("s1", 1)).await.unwrap();
    let mut batch = Vec::new();
    let big = serde_json::json!({ "text": "中".repeat(30_000) }).to_string();
    for i in 0..30 {
        let payload = if i == 10 {
            big.clone()
        } else {
            serde_json::json!({ "i": i }).to_string()
        };
        batch.push(event("s1", "text_delta", &payload));
    }
    store.append_events(&batch).await.unwrap();

    // Row cap: 10 rows per page, `more` set until drained.
    let page = store.events_page("s1", 0, 10, 10_000_000).await.unwrap();
    assert_eq!(page.events.len(), 10);
    assert!(page.more);
    assert_eq!(page.events[0].seq, Some(1));
    let last_seq = page.events.last().and_then(|e| e.seq).unwrap();
    let page2 = store
        .events_page("s1", last_seq, 10, 10_000_000)
        .await
        .unwrap();
    assert_eq!(page2.events.len(), 10);
    assert!(page2.more);

    // Byte budget: a small row still fits under the budget, and the page
    // stops before the oversized one without dropping it silently.
    let page = store.events_page("s1", 9, 5, 2048).await.unwrap();
    assert_eq!(page.events.last().and_then(|e| e.seq), Some(10));
    assert!(page.more, "the oversized row is signalled, not lost");
    // An oversized payload as the FIRST row becomes an omission marker that
    // still advances the cursor; the text is read via payload chunks.
    let page = store.events_page("s1", 10, 5, 2048).await.unwrap();
    assert!(!page.events.is_empty());
    let marker = &page.events[0];
    assert_eq!(marker.seq, Some(11));
    assert_eq!(marker.payload["omitted"], true);
    assert_eq!(marker.payload["read_via"], "event_payload");

    // Reassemble the omitted payload from bounded chunks.
    let chunk1 = store
        .event_payload_chunk("s1", 11, 0, 64 * 1024)
        .await
        .unwrap()
        .unwrap();
    assert!(
        chunk1.total_bytes > 64 * 1024,
        "oversized payload forces a second chunk"
    );
    assert_eq!(chunk1.bytes.len(), 64 * 1024);
    let rest = (chunk1.total_bytes - chunk1.bytes.len() as u64) as usize;
    let chunk2 = store
        .event_payload_chunk("s1", 11, chunk1.bytes.len() as u64, 64 * 1024)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(chunk2.bytes.len(), rest);
    let mut whole = chunk1.bytes.clone();
    whole.extend(chunk2.bytes.clone());
    assert_eq!(
        whole.len() as u64,
        chunk1.total_bytes,
        "chunks tile the payload exactly"
    );
    // An offset at the exact end is an empty slice; beyond it is rejected;
    // missing events are None.
    let at_end = store
        .event_payload_chunk("s1", 11, chunk1.total_bytes, 16)
        .await
        .unwrap()
        .unwrap();
    assert!(at_end.bytes.is_empty());
    assert!(store
        .event_payload_chunk("s1", 11, chunk1.total_bytes + 1, 16)
        .await
        .is_err());
    assert!(store
        .event_payload_chunk("s1", 999, 0, 16)
        .await
        .unwrap()
        .is_none());
}

fn task(id: &str) -> SubagentTaskRecord {
    SubagentTaskRecord {
        task_id: id.into(),
        parent_session_id: "parent".into(),
        child_session_id: format!("child-{id}"),
        parent_message_id: Some(format!("pm-{id}")),
        agent: "act".into(),
        prompt: format!("prompt-{id}"),
        result: None,
        status: SubagentStatus::Running,
        ok: None,
        started_at: 1,
        completed_at: None,
    }
}

#[tokio::test]
async fn subagent_task_lifecycle_and_parent_listing() {
    let store = mem().await;
    store.create_session(&meta_for("parent", 1)).await.unwrap();
    for id in ["t1", "t2", "t3"] {
        store.create_subagent_task(&task(id)).await.unwrap();
    }

    // Tasks start Running and are listed in insertion order.
    let listed = store.list_subagent_tasks("parent").await.unwrap();
    assert_eq!(
        listed.iter().map(|t| t.task_id.clone()).collect::<Vec<_>>(),
        vec!["t1", "t2", "t3"]
    );
    assert!(listed.iter().all(|t| t.status == SubagentStatus::Running));

    // Point lookup by task id resolves the parent session for resume.
    let got = store.get_subagent_task("t2").await.unwrap().unwrap();
    assert_eq!(got.parent_session_id, "parent");
    assert_eq!(got.child_session_id, "child-t2");
    assert!(store.get_subagent_task("nope").await.unwrap().is_none());
    assert!(store.list_subagent_tasks("nope").await.unwrap().is_empty());

    // Complete records the result and the ok flag.
    store
        .complete_subagent_task("t1", "all done", true)
        .await
        .unwrap();
    let done = store.get_subagent_task("t1").await.unwrap().unwrap();
    assert_eq!(done.status, SubagentStatus::Completed);
    assert_eq!(done.result.as_deref(), Some("all done"));
    assert_eq!(done.ok, Some(true));
    assert!(done.completed_at.is_some());

    // Failure keeps the result but flips the flag.
    store
        .complete_subagent_task("t2", "boom", false)
        .await
        .unwrap();
    let failed = store.get_subagent_task("t2").await.unwrap().unwrap();
    assert_eq!(failed.status, SubagentStatus::Failed);
    assert_eq!(failed.ok, Some(false));

    // Cancel records no result (the parent tool_use stays open).
    store.cancel_subagent_task("t3").await.unwrap();
    let cancelled = store.get_subagent_task("t3").await.unwrap().unwrap();
    assert_eq!(cancelled.status, SubagentStatus::Cancelled);
    assert_eq!(cancelled.result, None);

    // Terminal Completed/Failed states reject completion...
    assert!(store
        .complete_subagent_task("t1", "again", true)
        .await
        .is_err());
    // ...but cancel has NO status guard: the timeout handler legitimately
    // overrides a terminal state (Completed -> Cancelled).
    store.cancel_subagent_task("t1").await.unwrap();
    let overridden = store.get_subagent_task("t1").await.unwrap().unwrap();
    assert_eq!(overridden.status, SubagentStatus::Cancelled);
    // Completion resumes from Cancelled (the replay path).
    store
        .complete_subagent_task("t3", "replayed", true)
        .await
        .unwrap();
    let resumed = store.get_subagent_task("t3").await.unwrap().unwrap();
    assert_eq!(resumed.status, SubagentStatus::Completed);
    assert!(store.cancel_subagent_task("missing").await.is_err());

    // Deleting the PARENT session cascades the task rows...
    store.delete_session("parent").await.unwrap();
    assert!(store
        .list_subagent_tasks("parent")
        .await
        .unwrap()
        .is_empty());
    assert!(store.get_subagent_task("t1").await.unwrap().is_none());
    // ...and so does deleting the CHILD session (the other FK cascade).
    let store2 = mem().await;
    store2.create_session(&meta_for("parent", 1)).await.unwrap();
    store2.create_subagent_task(&task("t1")).await.unwrap();
    store2.delete_session("child-t1").await.unwrap();
    assert!(store2.get_subagent_task("t1").await.unwrap().is_none());
}
