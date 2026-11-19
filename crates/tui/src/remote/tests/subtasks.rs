//! Watcher terminal-path integration against the shared mock server: a
//! terminal `/index` status settles the target once with the server's status
//! string; summary/token fetch failures degrade to None/0 instead of failing
//! the round (the mock serves no `/result`/`/messages` routes), plus the
//! full completion writeback (status + `/result` summary + direct
//! `/messages` usage paging) into the parent registry and live chat.

use std::collections::HashMap;

use super::support::{Fixture, IndexReply};
use crate::{
    chat::ChatView,
    subtask_watcher::{run_with, spawn, WatchTarget},
    worker::UiEvent,
};
use opencoder_core::fleet::ExecutionStatus;
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

/// Agent/operator subtasks converge to the server-side `idle` status on
/// success (the worker never reports `done` for conversational kinds), so the
/// watcher must map `idle` to a `done` delivery and fold the same completion
/// writeback into the parent registry and live chat.
#[tokio::test]
async fn watcher_maps_idle_agent_convergence_to_done() {
    let fixture = Fixture::new().await;
    {
        let mut state = fixture.state.lock().unwrap();
        state.status = "idle".into();
        state.result_summary = Some("空闲收敛结果".into());
        // Two assistant messages whose persisted usage sums to 90: the
        // watcher's `fetch_tokens` pages the DIRECT /messages route.
        let mut first = Message::assistant("m1");
        first.usage.total_tokens = 50;
        let mut second = Message::assistant("m2");
        second.usage.total_tokens = 40;
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
    assert_eq!(status, "done", "idle convergence is delivered as done");
    assert!(
        summary
            .as_deref()
            .is_some_and(|text| text.contains("空闲收敛结果")),
        "summary came from /result, got: {summary:?}"
    );
    assert_eq!(tokens, 90, "paged message usage 50 + 40 folds");
    assert!(rx.try_recv().is_err(), "settled target reports once");
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
        Some((1, 1, 90)),
        "mapped delivery reports (done, total, delta)"
    );
    assert_eq!(chat.tokens_total, 90, "delta folds into [tok cost]");
    assert_eq!(chat.subtask_done, 1);
    assert_eq!(chat.subtask_total, 1);
    let registry = crate::remote::subtasks::load_registry(&store, "parent-1")
        .await
        .unwrap();
    let entry =
        crate::subtask_registry::find(&registry, "agent-sub-1").expect("registry kept the entry");
    assert_eq!(entry.status, "done");
    assert_eq!(entry.tokens, 90);
    assert_eq!(
        entry.result_summary.as_deref(),
        Some("空闲收敛结果"),
        "stored summary is the bounded /result copy"
    );
}

/// `/subtasks` cancel asks the server to CANCEL the execution (the old
/// interrupt action left interrupted runs unsettled locally).
#[tokio::test]
async fn cancel_subtask_uses_cancel_command() {
    let fixture = Fixture::new().await;
    let client = crate::remote::client::ServerClient::new(&fixture.url, None).unwrap();
    let outcome = crate::remote::subtasks::cancel(&client, "agent-sub-1")
        .await
        .unwrap();
    assert_eq!(
        outcome.status, None,
        "the legacy {{ok:true}} body carries no terminal status"
    );
    let requests = fixture.state.lock().unwrap().requests.clone();
    let (request, body) = requests
        .iter()
        .find(|(request, _)| request.ends_with("/api/executions/agent-sub-1/commands"))
        .expect("cancel command reached the mock");
    assert_eq!(request, "POST /api/executions/agent-sub-1/commands");
    assert_eq!(body["action"], "cancel");
    assert_ne!(body["action"], "interrupt");
}

/// A cancel response carrying a terminal `status` is surfaced so the caller
/// can settle the registry without waiting for another watcher round.
#[tokio::test]
async fn cancel_returns_cancelled_status() {
    let fixture = Fixture::new().await;
    fixture.state.lock().unwrap().command_status = Some("cancelled".into());
    let client = crate::remote::client::ServerClient::new(&fixture.url, None).unwrap();
    let outcome = crate::remote::subtasks::cancel(&client, "agent-sub-1")
        .await
        .unwrap();
    assert_eq!(outcome.status, Some(ExecutionStatus::Cancelled));
}

/// Missing `status` in the cancel response keeps the watcher observing.
#[tokio::test]
async fn cancel_without_status_is_not_terminal() {
    let fixture = Fixture::new().await;
    let client = crate::remote::client::ServerClient::new(&fixture.url, None).unwrap();
    let outcome = crate::remote::subtasks::cancel(&client, "agent-sub-1")
        .await
        .unwrap();
    assert!(outcome.status.is_none());
}

/// A scripted target helper (`session` = "parent-1").
fn watch(execution_id: &str) -> WatchTarget {
    WatchTarget {
        session: "parent-1".into(),
        execution_id: execution_id.into(),
    }
}

fn script_status(status: &'static str, rounds: usize) -> Vec<IndexReply> {
    vec![IndexReply::Status(status); rounds]
}

/// Receive `count` subtask updates as `id -> status`.
async fn collect_updates(
    rx: &mut tokio::sync::mpsc::Receiver<UiEvent>,
    count: usize,
) -> HashMap<String, String> {
    let mut updates = HashMap::new();
    for _ in 0..count {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("watcher update timeout")
            .unwrap();
        let UiEvent::SubtaskUpdate { id, status, .. } = event else {
            panic!("expected a subtask update, got: {event:?}");
        };
        updates.insert(id, status);
    }
    updates
}

/// `interrupted` is locally terminal: one delivery with that status, no
/// re-poll, and the watcher exits.
#[tokio::test]
async fn watcher_settles_interrupted_status_once() {
    let fixture = Fixture::new().await;
    fixture
        .state
        .lock()
        .unwrap()
        .index_script
        .insert("agent-sub-1".into(), script_status("interrupted", 1));
    let client = crate::remote::client::ServerClient::new(&fixture.url, None).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let handle = spawn(client, tx, vec![watch("agent-sub-1")]);
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("watcher update timeout")
        .unwrap();
    let UiEvent::SubtaskUpdate { id, status, .. } = event else {
        panic!("expected a subtask update, got: {event:?}");
    };
    assert_eq!(id, "agent-sub-1");
    assert_eq!(status, "interrupted");
    assert!(
        rx.try_recv().is_err(),
        "interrupted target is not re-polled"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), handle)
            .await
            .is_ok(),
        "watcher exits once the interrupted target settles"
    );
}

/// One permanently failing target settles as error on its own fifth failed
/// poll while a healthy sibling keeps polling to `done`.
#[tokio::test]
async fn watcher_failing_target_does_not_error_healthy_pending_target() {
    let fixture = Fixture::new().await;
    {
        let mut state = fixture.state.lock().unwrap();
        state
            .index_script
            .insert("agent-bad".into(), vec![IndexReply::Fail]);
        let mut good = script_status("running", 6);
        good.push(IndexReply::Status("done"));
        state.index_script.insert("agent-good".into(), good);
    }
    let client = crate::remote::client::ServerClient::new(&fixture.url, None).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let handle = tokio::spawn(run_with(
        client,
        tx,
        vec![watch("agent-bad"), watch("agent-good")],
        std::time::Duration::from_millis(10),
    ));
    let updates = collect_updates(&mut rx, 2).await;
    assert_eq!(updates.get("agent-bad").map(String::as_str), Some("error"));
    assert_eq!(
        updates.get("agent-good").map(String::as_str),
        Some("done"),
        "healthy target must never be dragged into error"
    );
    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.index_calls.get("agent-bad").copied(), Some(5));
        assert_eq!(state.index_calls.get("agent-good").copied(), Some(7));
    }
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), handle)
            .await
            .is_ok(),
        "watcher exits after every target settles"
    );
}

/// A healthy poll resets the target's failure streak: four fails, one
/// running, one fail, then done never settles early.
#[tokio::test]
async fn watcher_resets_target_failure_streak_on_a_healthy_poll() {
    let fixture = Fixture::new().await;
    {
        let mut state = fixture.state.lock().unwrap();
        state.index_script.insert(
            "agent-flaky".into(),
            vec![
                IndexReply::Fail,
                IndexReply::Fail,
                IndexReply::Fail,
                IndexReply::Fail,
                IndexReply::Status("running"),
                IndexReply::Fail,
                IndexReply::Status("done"),
            ],
        );
        state
            .index_script
            .insert("agent-poison".into(), vec![IndexReply::Fail]);
    }
    let client = crate::remote::client::ServerClient::new(&fixture.url, None).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let handle = tokio::spawn(run_with(
        client,
        tx,
        vec![watch("agent-flaky"), watch("agent-poison")],
        std::time::Duration::from_millis(10),
    ));
    let updates = collect_updates(&mut rx, 2).await;
    assert_eq!(
        updates.get("agent-flaky").map(String::as_str),
        Some("done"),
        "a healthy poll resets the failed-poll streak"
    );
    assert_eq!(
        updates.get("agent-poison").map(String::as_str),
        Some("error")
    );
    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.index_calls.get("agent-flaky").copied(), Some(7));
        assert_eq!(state.index_calls.get("agent-poison").copied(), Some(5));
    }
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), handle)
            .await
            .is_ok(),
        "watcher exits after every target settles"
    );
}
