//! Integration tests for the TUI-sidecar actor (`sidecar_ui::spawn_actor`):
//! lazy conversation build, follow-up continuity, zero sidecar persistence,
//! bare-`LlmUsage` cost accounting to the main session and the destroy
//! semantics of [`SidecarCmd::Reset`] (idle + in-flight abort).

use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, Config, Message};
use opencoder_llm::{CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, Store};
use tokio::sync::mpsc;

use crate::sidecar_ui::{spawn_actor, SidecarCmd};
use crate::worker::UiEvent;

const SID: &str = "actor-sid";

fn done(text: &str, total: u64) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: Vec::<CompletedToolCall>::new(),
        usage: Some(Usage {
            input_tokens: total - 1,
            output_tokens: 1,
            total_tokens: total,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        }),
    }
}

/// In-memory store seeded with a two-message main transcript (the sidecar's
/// background context) and its session row (events FK on sessions).
async fn seeded_store() -> Arc<dyn Store> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&opencoder_store::SessionMeta {
            id: SID.into(),
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .append_messages(
            SID,
            &[
                Message::user("m1", "主任务背景 alpha"),
                Message::user("m2", "主任务背景 beta"),
            ],
        )
        .await
        .unwrap();
    store
}

/// Collect `UiEvent::Session` payloads into a shared buffer.
fn collector(mut evt_rx: mpsc::Receiver<UiEvent>) -> Arc<Mutex<Vec<SessionEvent>>> {
    let seen: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    tokio::spawn(async move {
        while let Some(ev) = evt_rx.recv().await {
            if let UiEvent::Session(sev) = ev {
                sink.lock().unwrap().push(sev);
            }
        }
    });
    seen
}

async fn wait_until<F: Fn() -> bool>(desc: &str, pred: F) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while !pred() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timeout waiting for {desc}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

fn sidecar_turns(events: &[SessionEvent]) -> Vec<(bool, String, u64)> {
    events
        .iter()
        .filter_map(|ev| match ev {
            SessionEvent::SidecarTurn {
                ok,
                answer,
                total_tokens,
                ..
            } => Some((*ok, answer.clone(), *total_tokens)),
            _ => None,
        })
        .collect()
}

fn sidecar_starts(events: &[SessionEvent]) -> usize {
    events
        .iter()
        .filter(|ev| matches!(ev, SessionEvent::SidecarStart { .. }))
        .count()
}

/// Spawn the actor; returns the command sender plus the shared event buffer.
fn actor(
    mock: Arc<MockChatClient>,
    store: Arc<dyn Store>,
) -> (mpsc::Sender<SidecarCmd>, Arc<Mutex<Vec<SessionEvent>>>) {
    let session = SessionState::new(
        SID,
        resolve_agent("act").unwrap(),
        Config::default(),
        mock.clone(),
        std::env::temp_dir(),
    );
    let (evt_tx, evt_rx) = mpsc::channel::<UiEvent>(256);
    let seen = collector(evt_rx);
    let ask = spawn_actor(&session, evt_tx, Some(store.clone()));
    (ask, seen)
}

/// Poll until the main session carries `expected` persisted llm_usage rows
/// (the actor persists after emitting the Turn frame, so a plain read can
/// race the write).
async fn wait_for_usage_rows(store: &Arc<dyn Store>, expected: usize) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if llm_usage_row_count(store).await >= expected {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timeout waiting for {expected} llm_usage rows"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Persisted llm_usage row count on the main session.
async fn llm_usage_row_count(store: &Arc<dyn Store>) -> usize {
    store
        .events_after(SID, 0)
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| r.sse_kind.as_deref() == Some("llm_usage"))
        .count()
}

#[tokio::test]
async fn sidecar_actor_answers_follow_ups_without_persisting_content() {
    let store = seeded_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![
                LlmEvent::TextDelta("旁路答案一".into()),
                done("旁路答案一", 7),
            ])
            .push_script(vec![done("旁路答案二", 13)]),
    );
    let (ask, seen) = actor(mock.clone(), store.clone());

    // Question 1: lazily builds the conversation (snapshot + SidecarStart).
    ask.send(SidecarCmd::Ask("第一个问题?".into()))
        .await
        .unwrap();
    wait_until("first SidecarTurn", || {
        !sidecar_turns(&seen.lock().unwrap()).is_empty()
    })
    .await;

    // Question 2: the SAME conversation continues (exactly one SidecarStart).
    ask.send(SidecarCmd::Ask("第二个问题?".into()))
        .await
        .unwrap();
    wait_until("second SidecarTurn", || {
        sidecar_turns(&seen.lock().unwrap()).len() >= 2
    })
    .await;
    drop(ask);

    let events = seen.lock().unwrap().clone();
    assert_eq!(
        sidecar_starts(&events),
        1,
        "one conversation across follow-ups"
    );

    let turns = sidecar_turns(&events);
    assert_eq!(turns.len(), 2);
    assert!(turns[0].1.contains("旁路答案一"));
    assert!(turns[1].1.contains("旁路答案二"));

    // Snapshot-in: request 1 carries the seeded main transcript.
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2, "one LLM round per question");
    let first = serde_json::to_string(&reqs[0].messages).unwrap();
    assert!(
        first.contains("主任务背景 alpha"),
        "snapshot must seed context"
    );
    // Continuity: request 2 sees the first Q/A pair from memory.
    let second = serde_json::to_string(&reqs[1].messages).unwrap();
    assert!(
        second.contains("第一个问题?"),
        "first question is in follow-up ctx"
    );
    assert!(
        second.contains("旁路答案一"),
        "first answer is in follow-up ctx"
    );
    assert!(second.contains("第二个问题?"));

    // Zero content persistence: message rows unchanged.
    let msgs = store.load_messages(SID).await.unwrap();
    assert_eq!(msgs.len(), 2, "sidecar Q/A must never write message rows");

    // Cost accounting: exactly the two bare LlmUsage events persist; none of
    // the Sidecar* display frames do.
    wait_for_usage_rows(&store, 2).await;
    let rows = store.events_after(SID, 0).await.unwrap();
    let kinds: Vec<&str> = rows
        .iter()
        .map(|r| r.sse_kind.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(
        kinds.iter().filter(|k| **k == "llm_usage").count(),
        2,
        "both turns' usage lands on the main session, got {kinds:?}"
    );
    assert!(
        !kinds.iter().any(|k| k.starts_with("sidecar_")),
        "sidecar display frames never persist, got {kinds:?}"
    );
}

#[tokio::test]
async fn sidecar_reset_idle_destroys_conversation_next_ask_rebuilds_fresh() {
    let store = seeded_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done("答案一", 7)])
            .push_script(vec![done("答案二", 9)]),
    );
    let (ask, seen) = actor(mock.clone(), store.clone());

    ask.send(SidecarCmd::Ask("第一个问题?".into()))
        .await
        .unwrap();
    wait_until("first SidecarTurn", || {
        !sidecar_turns(&seen.lock().unwrap()).is_empty()
    })
    .await;

    // Idle Reset: the conversation is dropped.
    ask.send(SidecarCmd::Reset).await.unwrap();

    // Next Ask rebuilds from a FRESH snapshot: a second SidecarStart, and
    // the follow-up context carries NO trace of the first Q/A pair.
    ask.send(SidecarCmd::Ask("第二个问题?".into()))
        .await
        .unwrap();
    wait_until("second SidecarTurn", || {
        sidecar_turns(&seen.lock().unwrap()).len() >= 2
    })
    .await;
    drop(ask);

    let events = seen.lock().unwrap().clone();
    assert_eq!(sidecar_starts(&events), 2, "Reset forces a fresh conv");

    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2, "one LLM round per question");
    let second = serde_json::to_string(&reqs[1].messages).unwrap();
    assert!(
        !second.contains("第一个问题?"),
        "destroyed conversation must not leak into the rebuilt one"
    );
    assert!(
        !second.contains("答案一"),
        "destroyed conversation's answer must not leak"
    );
    assert!(
        second.contains("主任务背景 alpha"),
        "rebuild still starts from the store snapshot"
    );

    // Both turns' usage still lands (accounting is per-turn, not per-conv).
    wait_for_usage_rows(&store, 2).await;
}

#[tokio::test]
async fn sidecar_reset_aborts_inflight_turn_no_content_frames() {
    let store = seeded_store().await;
    let notify = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(MockChatClient::new().push_hang(notify.clone()));
    let (ask, seen) = actor(mock.clone(), store.clone());

    ask.send(SidecarCmd::Ask("会被中止的问题?".into()))
        .await
        .unwrap();
    wait_until("turn starts", || mock.call_count() >= 1).await;

    // Destroy mid-flight: the actor aborts the turn task.
    ask.send(SidecarCmd::Reset).await.unwrap();
    notify.notify_waiters(); // unblock the hung stream if it survived
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let events = seen.lock().unwrap().clone();
    assert!(
        sidecar_turns(&events).is_empty(),
        "aborted turn must not emit a SidecarTurn frame"
    );
    assert_eq!(
        llm_usage_row_count(&store).await,
        0,
        "aborted turn produced no usage: nothing to persist"
    );

    // The actor survives the abort: the next Ask runs on a REBUILT conv
    // (fresh snapshot, a second SidecarStart, no trace of the aborted Q).
    mock.queue_script(vec![done("复活答案", 5)]);
    ask.send(SidecarCmd::Ask("新问题?".into())).await.unwrap();
    wait_until("post-abort SidecarTurn", || {
        !sidecar_turns(&seen.lock().unwrap()).is_empty()
    })
    .await;
    drop(ask);

    let events = seen.lock().unwrap().clone();
    assert_eq!(sidecar_starts(&events), 2, "abort forces a fresh conv");
    let turns = sidecar_turns(&events);
    assert_eq!(turns.len(), 1);
    assert!(turns[0].1.contains("复活答案"));

    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2);
    let second = serde_json::to_string(&reqs[1].messages).unwrap();
    assert!(
        !second.contains("会被中止的问题?"),
        "aborted question must not leak into the rebuilt conv"
    );
    wait_for_usage_rows(&store, 1).await;
}

/// Defect-A guard: a follow-up that raced into the actor's backlog while a
/// turn was in flight must DIE with the panel. `Reset` (ESC / Ctrl+L /
/// re-entry) aborts the in-flight turn AND discards the backlog — a queued
/// question must never rebuild the conversation and keep burning tokens
/// after the user left the panel.
#[tokio::test]
async fn sidecar_reset_discards_backlogged_follow_ups() {
    let store = seeded_store().await;
    let notify = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(MockChatClient::new().push_hang(notify.clone()));
    let (ask, seen) = actor(mock.clone(), store.clone());

    ask.send(SidecarCmd::Ask("在飞的问题?".into()))
        .await
        .unwrap();
    wait_until("turn starts", || mock.call_count() >= 1).await;

    // This Ask lands in the racing loop's backlog (channel order keeps it
    // strictly ahead of the Reset below).
    ask.try_send(SidecarCmd::Ask("排队的问题?".into())).unwrap();
    // Destroy: aborts the in-flight turn AND drops the queued follow-up.
    ask.send(SidecarCmd::Reset).await.unwrap();
    notify.notify_waiters(); // unblock the hung stream if it survived
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // The queued question never ran: no second LLM call, no Turn frame.
    assert_eq!(
        mock.call_count(),
        1,
        "backlogged question must not run after Reset"
    );
    assert_eq!(
        mock.requests().len(),
        1,
        "no LLM request may carry the discarded question"
    );
    let turns = sidecar_turns(&seen.lock().unwrap());
    assert!(
        turns.is_empty(),
        "neither the aborted nor the discarded question may emit a Turn frame"
    );
    assert_eq!(
        llm_usage_row_count(&store).await,
        0,
        "destroyed panel must not accrue any usage"
    );

    // The actor survives: the next Ask rebuilds a fresh conversation.
    mock.queue_script(vec![done("重建后的答案", 5)]);
    ask.send(SidecarCmd::Ask("重建后的问题?".into()))
        .await
        .unwrap();
    wait_until("post-reset SidecarTurn", || {
        !sidecar_turns(&seen.lock().unwrap()).is_empty()
    })
    .await;
    drop(ask);

    let events = seen.lock().unwrap().clone();
    assert_eq!(sidecar_starts(&events), 2, "Reset forces a fresh conv");
    assert_eq!(sidecar_turns(&events).len(), 1);
    assert!(sidecar_turns(&events)[0].1.contains("重建后的答案"));
    wait_for_usage_rows(&store, 1).await;
}
