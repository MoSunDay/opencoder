//! Integration test: the `QueueConsumed` event carries the consumed prompt's
//! text so display surfaces can echo it at the exact activation instant, and
//! it arrives in the event stream *before* the `TextDelta`s of the turn it
//! triggers (the echo precedes the output, not the other way around).

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

/// A mock script that streams a text delta then completes — produces a real
/// `TextDelta` event the runner forwards to `on_event`.
fn stream_turn(text: &str) -> Vec<LlmEvent> {
    vec![
        LlmEvent::TextDelta(text.into()),
        LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: Some(Usage::default()),
        },
    ]
}

async fn seed(store: &Arc<dyn Store>, id: &str, agent: &str) {
    store
        .create_session(&opencoder_store::SessionMeta {
            id: id.into(),
            agent: Some(agent.into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

fn mk_input(session_id: &str, delivery: Delivery, prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: opencoder_session::runner::new_id(),
        session_id: session_id.into(),
        delivery,
        prompt: prompt.into(),
        images: vec![],
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

/// A queued prompt is drained at an idle boundary. The `QueueConsumed` event
/// must (a) carry the prompt text and (b) appear before the `TextDelta`s of
/// the turn it triggers.
#[tokio::test]
async fn queue_consumed_carries_text_and_precedes_output() {
    let store = mem_store().await;
    seed(&store, "echo-sess", "act").await;

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(stream_turn("kickoff reply"))
            .push_script(stream_turn("queued reply")),
    ) as Arc<dyn ChatStream>;

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "echo-sess",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input("echo-sess", Delivery::Queue, "queued task"))
        .await
        .unwrap();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    let evs = events.lock().unwrap();

    // (a) QueueConsumed carries the exact prompt text.
    let qc_idx = evs.iter().position(|e| {
        matches!(e, SessionEvent::QueueConsumed { text, .. } if text == "queued task")
    });
    assert!(
        qc_idx.is_some(),
        "QueueConsumed must carry the prompt text \"queued task\""
    );

    // (b) It precedes the TextDelta of the turn it triggered.
    let td_idx = evs.iter().position(|e| {
        matches!(e, SessionEvent::TextDelta(t) if t.contains("queued reply"))
    });
    assert!(
        td_idx.is_some(),
        "expected a TextDelta for the queued-reply turn"
    );
    assert!(
        qc_idx.unwrap() < td_idx.unwrap(),
        "QueueConsumed must arrive before the TextDelta it triggers"
    );
}

/// A compound queued prompt (`/plan <text>`) emits `QueueConsumed` carrying
/// the *raw* text so the echo matches what the user typed, while the tail
/// still reaches the LLM in the new mode.
#[tokio::test]
async fn queue_consumed_compound_carries_raw_text() {
    let store = mem_store().await;
    seed(&store, "echo-cmp", "act").await;

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(stream_turn("kickoff"))
            .push_script(stream_turn("plan reply")),
    ) as Arc<dyn ChatStream>;

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "echo-cmp",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input("echo-cmp", Delivery::Queue, "/plan review the code"))
        .await
        .unwrap();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    let evs = events.lock().unwrap();

    let carries = evs.iter().any(|e| {
        matches!(e, SessionEvent::QueueConsumed { text, .. } if text == "/plan review the code")
    });
    assert!(
        carries,
        "QueueConsumed must carry the raw compound text \"/plan review the code\""
    );

    let has_reply = evs
        .iter()
        .any(|e| matches!(e, SessionEvent::TextDelta(t) if t.contains("plan reply")));
    assert!(has_reply, "the /plan <text> tail must produce an LLM turn");
}
