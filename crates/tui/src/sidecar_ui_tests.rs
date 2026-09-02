//! Integration tests for the TUI-sidecar actor (`sidecar_ui::spawn_actor`):
//! lazy conversation build, follow-up continuity, zero sidecar persistence
//! and bare-`LlmUsage` cost accounting to the main session.

use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, Config, Message};
use opencoder_llm::{CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, Store};
use tokio::sync::mpsc;

use crate::sidecar_ui::spawn_actor;
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

    // Question 1: lazily builds the conversation (snapshot + SidecarStart).
    ask.send("第一个问题?".into()).await.unwrap();
    wait_until("first SidecarTurn", || {
        !sidecar_turns(&seen.lock().unwrap()).is_empty()
    })
    .await;

    // Question 2: the SAME conversation continues (exactly one SidecarStart).
    ask.send("第二个问题?".into()).await.unwrap();
    wait_until("second SidecarTurn", || {
        sidecar_turns(&seen.lock().unwrap()).len() >= 2
    })
    .await;
    drop(ask);

    let events = seen.lock().unwrap().clone();
    let starts = events
        .iter()
        .filter(|ev| matches!(ev, SessionEvent::SidecarStart { .. }))
        .count();
    assert_eq!(starts, 1, "one conversation across follow-ups");

    let turns = sidecar_turns(&events);
    assert_eq!(turns.len(), 2);
    assert!(turns.iter().all(|(ok, _, _)| *ok), "both turns succeed");
    assert_eq!(turns[0].1, "旁路答案一", "turn answer is the round's text");
    assert_eq!(turns[0].2, 7, "per-turn usage total");
    assert_eq!(turns[1].2, 13);
    let id0 = match &events[0] {
        SessionEvent::SidecarStart { id, question } => {
            assert_eq!(question, "第一个问题?");
            assert!(id.starts_with("sidecar-"), "sidecar id prefix, got {id}");
            id.clone()
        }
        other => panic!("first event must be SidecarStart, got {other:?}"),
    };
    assert!(events.iter().all(|ev| match ev {
        SessionEvent::SidecarTurn { id, .. } => id == &id0,
        SessionEvent::SidecarStart { .. } => true,
        _ => true,
    }));

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
