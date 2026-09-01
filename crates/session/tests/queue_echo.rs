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

/// A bare control command consumed from the queue is applied inline: it
/// echoes nothing (empty `QueueConsumed` text), triggers no LLM turn and
/// records no user message — the command token itself never reaches the
/// transcript or the context.
#[tokio::test]
async fn bare_control_command_queues_silently() {
    let store = mem_store().await;
    seed(&store, "echo-bare", "act").await;

    // No scripted turn: any LLM call would fail the test loudly.
    let mock = Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>;

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "echo-bare",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input("echo-bare", Delivery::Queue, "/plan"))
        .await
        .unwrap();

    // Empty initial prompt: drain mode claims the bare command at the top of
    // run_loop, so the mock is never called.
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, String::new(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    let evs = events.lock().unwrap();
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::QueueConsumed { text, .. } if text.is_empty())),
        "a bare control command must echo nothing (empty QueueConsumed text)"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "plan")),
        "the switch still applies"
    );
    assert!(
        !evs.iter().any(|e| matches!(e, SessionEvent::TextDelta(_))),
        "no LLM turn for a bare control command"
    );
    assert!(
        !session
            .messages
            .iter()
            .any(|m| m.role == opencoder_core::Role::User),
        "nothing recorded into context"
    );
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
    let qc_idx = evs.iter().position(
        |e| matches!(e, SessionEvent::QueueConsumed { text, .. } if text == "queued task"),
    );
    assert!(
        qc_idx.is_some(),
        "QueueConsumed must carry the prompt text \"queued task\""
    );

    // (b) It precedes the TextDelta of the turn it triggered.
    let td_idx = evs
        .iter()
        .position(|e| matches!(e, SessionEvent::TextDelta(t) if t.contains("queued reply")));
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
async fn queue_consumed_compound_carries_tail_text() {
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
        .admit_input(&mk_input(
            "echo-cmp",
            Delivery::Queue,
            "/plan review the code",
        ))
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

    // The echo is model-facing: only the tail (what record_compound records
    // into context), never the `/plan` token itself.
    let carries = evs.iter().any(|e| {
        matches!(e, SessionEvent::QueueConsumed { text, .. } if text == "review the code")
    });
    assert!(
        carries,
        "QueueConsumed must carry the compound tail \"review the code\", not the raw command"
    );
    let raw_leak = evs
        .iter()
        .any(|e| matches!(e, SessionEvent::QueueConsumed { text, .. } if text.contains("/plan")));
    assert!(!raw_leak, "the /plan token must never be echoed");

    let has_reply = evs
        .iter()
        .any(|e| matches!(e, SessionEvent::TextDelta(t) if t.contains("plan reply")));
    assert!(
        has_reply,
        "the /plan <text> tail must produce an LLM turn"
    );
}
