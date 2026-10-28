//! Watcher terminal-path integration against the shared mock server: a
//! terminal `/index` status settles the target once with the server's status
//! string; summary/token fetch failures degrade to None/0 instead of failing
//! the round (the mock serves no `/result`/`/messages` routes), plus the
//! full completion writeback (status + `/result` summary + direct
//! `/messages` usage paging) into the parent registry and live chat.

use std::collections::HashMap;

use super::support::Fixture;
use crate::{
    chat::ChatView,
    subtask_watcher::{spawn, WatchTarget},
    worker::UiEvent,
};
use opencoder_core::harness::{HarnessRuntime, SubtaskRef};
use opencoder_core::Message;
use opencoder_store::SessionMeta;

// Real time (no paused clock): the first poll round fires immediately and
// the mock answers over a real socket; only the round cadence would be slow.
#[tokio::test]
async fn terminal_subtask_settles_once_with_server_status() {
    let fixture = Fixture::new().await;
    fixture.state.lock().unwrap().status = "done".into();
    let client = crate::remote::client::ServerClient::new(&fixture.url, None).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let handle = spawn(
        client,
        tx,
        vec![WatchTarget {
            session: "parent-1".into(),
            execution_id: "agent-sub-1".into(),
        }],
    );
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("watcher update timeout")
        .unwrap();
    let UiEvent::SubtaskUpdate {
        session,
        id,
        status,
        summary,
        tokens,
    } = event
    else {
        panic!("expected a subtask update, got: {event:?}");
    };
    assert_eq!((session.as_str(), id.as_str()), ("parent-1", "agent-sub-1"));
    assert_eq!(status, "done");
    // The mock has no /result route: the summary degrades to None.
    assert_eq!(summary, None);
    assert_eq!(tokens, 0);
    assert!(rx.try_recv().is_err(), "terminal target is not re-polled");
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), handle)
            .await
            .is_ok(),
        "watcher exits once no targets remain"
    );
}

/// End-to-end watcher writeback against the shared mock server: a terminal
/// `/index` status plus a real `/result` summary and direct `/messages` usage
/// paging must land as ONE `SubtaskUpdate` whose fold (`subtask_events::
/// apply_update`) writes the parent registry AND the live chat counters, and
/// whose redelivery is a no-op (idempotence).
#[tokio::test]
async fn watcher_completion_folds_tokens_into_parent_registry_and_chat() {
    let fixture = Fixture::new().await;
    {
        let mut state = fixture.state.lock().unwrap();
        state.status = "done".into();
        state.result_summary = Some("答案文本".into());
        // Two assistant messages whose persisted usage sums to 200: the
        // watcher's `fetch_tokens` pages the DIRECT /messages route.
        let mut first = Message::assistant("m1");
        first.usage.total_tokens = 120;
        let mut second = Message::assistant("m2");
        second.usage.total_tokens = 80;
        state.messages = vec![first, second];
    }
    // Parent session row + runtime registry owning a pending subtask (the
    // state `@capability` dispatch leaves behind after registration).
    let store = fixture.store.clone();
    store
        .create_session(&SessionMeta {
            id: "parent-1".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .set_harness_runtime(
            "parent-1",
            &HarnessRuntime {
                subtasks: vec![SubtaskRef {
                    execution_id: "agent-sub-1".into(),
                    kind: "agent".into(),
                    capability_id: "ops_x".into(),
                    question: "check the logs".into(),
                    status: "pending".into(),
                    result_summary: None,
                    tokens: 0,
                    created_at: 1,
                }],
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let client = crate::remote::client::ServerClient::new(&fixture.url, None).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let handle = spawn(
        client,
        tx,
        vec![WatchTarget {
            session: "parent-1".into(),
            execution_id: "agent-sub-1".into(),
        }],
    );
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("watcher update timeout")
        .unwrap();
    let UiEvent::SubtaskUpdate {
        session,
        id,
        status,
        summary,
        tokens,
    } = event
    else {
        panic!("expected a subtask update, got: {event:?}");
    };
    assert_eq!((session.as_str(), id.as_str()), ("parent-1", "agent-sub-1"));
    assert_eq!(status, "done");
    assert!(
        summary
            .as_deref()
            .is_some_and(|text| text.contains("答案文本")),
        "summary came from /result, got: {summary:?}"
    );
    assert_eq!(tokens, 200, "paged message usage 120 + 80 folds");
    assert!(rx.try_recv().is_err(), "terminal target reports once");
    handle.abort();

    // Fold into the live parent session: registry writeback + chat counters.
    let mut session_states = HashMap::new();
    let mut chat = ChatView::default();
    let outcome = crate::subtask_events::apply_update(
        &store,
        &mut session_states,
        "parent-1",
        &mut chat,
        "parent-1",
        &id,
        &status,
        summary.as_deref(),
        tokens,
    )
    .await;
    assert_eq!(
        outcome,
        Some((1, 1, 200)),
        "first delivery reports (done, total, delta)"
    );
    assert_eq!(chat.tokens_total, 200, "delta folds into [tok cost]");
    assert_eq!(chat.subtask_done, 1);
    assert_eq!(chat.subtask_total, 1);
    let registry = crate::remote::subtasks::load_registry(&store, "parent-1")
        .await
        .unwrap();
    let entry =
        crate::subtask_registry::find(&registry, "agent-sub-1").expect("registry kept the entry");
    assert_eq!(entry.status, "done");
    assert_eq!(entry.tokens, 200);
    assert_eq!(
        entry.result_summary.as_deref(),
        Some("答案文本"),
        "stored summary is the bounded /result copy"
    );

    // Redelivering the SAME event must not move anything.
    let repeat = crate::subtask_events::apply_update(
        &store,
        &mut session_states,
        "parent-1",
        &mut chat,
        "parent-1",
        &id,
        &status,
        summary.as_deref(),
        tokens,
    )
    .await;
    assert_eq!(repeat, None, "idempotent redelivery reports no change");
    assert_eq!(chat.tokens_total, 200, "[tok cost] does not double-count");
}
