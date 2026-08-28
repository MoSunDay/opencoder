//! Regression tests for `/clear_context`: a plain last assistant reply must
//! survive as a NEUTRAL continuity seed — never wiped, never repackaged as an
//! execution directive. Self-contained (does not depend on helpers elsewhere).

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: Some(Usage::default()),
    }
}

async fn seed(store: &Arc<dyn Store>, id: &str, agent: &str) {
    store
        .create_session(&SessionMeta {
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

/// Contract: `/clear_context` must NEVER fully wipe — the last assistant
/// reply ("task done") survives as a NEUTRAL continuity seed. The
/// fabrication guard still holds: the reply is NOT wrapped in the
/// "Execute it now" directive; it reaches the model only as prior context.
/// The run continues (the seed falls through to an LLM turn), unlike the
/// blank-sentinel stop.
#[tokio::test]
async fn clear_context_seeds_last_say_not_fabricated_directive() {
    let store = mem_store().await;
    seed(&store, "act-no-plan", "act").await;

    // History whose last assistant text is a plain completion. The old
    // `handoff` picked any last say up and wrapped it in the
    // "Planning phase complete. ... Execute it now" directive; the seed path
    // now carries it forward as plain context instead.
    let msgs = vec![Message::user("u1", "implement X"), {
        let mut m = Message::assistant("a1");
        m.blocks.push(ContentBlock::text("task done"));
        m
    }];
    store.append_messages("act-no-plan", &msgs).await.unwrap();

    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("continuing from the seed")]));
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "act-no-plan",
        resolve_agent("act").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = msgs.clone();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "/clear_context".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    // Seed path: transcript collapses to the seed message (synthetic user),
    // then the execution turn appends the reply.
    assert!(
        session.messages[0].text().contains("task done"),
        "seed carries the last say as prior context: {}",
        session.messages[0].text()
    );
    assert!(
        session.messages[0].text().contains("prior context"),
        "seed uses the neutral continuity wrapper: {}",
        session.messages[0].text()
    );
    assert!(
        !session.messages[0].text().contains("Execute it now"),
        "seed must NOT use an execution directive prefix"
    );
    assert_eq!(
        session.handoff_plan.as_deref(),
        Some("<<OPENCODER_CLEAR_SEED>>task done"),
        "seed marker stored so resume reconstructs the seed"
    );
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.role == Role::Assistant && m.text().contains("continuing from the seed")),
        "seed is executed, not stranded"
    );

    // The model saw the last say as context — never a fabricated execution
    // directive, never the raw marker.
    let requests = mock.requests();
    assert_eq!(requests.len(), 1, "one LLM call to continue from the seed");
    let body = requests[0].to_body().to_string();
    assert!(
        body.contains("task done"),
        "last say reaches the model: {body}"
    );
    assert!(
        !body.contains("<<OPENCODER_CLEAR_SEED>>"),
        "raw seed marker must never reach the model: {body}"
    );
    assert!(
        !body.contains("Execute it now"),
        "no fabricated execution directive: {body}"
    );

    let evs = events.lock().unwrap();
    assert!(
        evs.iter()
            .any(|e| matches!(e, opencoder_session::SessionEvent::TranscriptReset(_))),
        "TranscriptReset emitted"
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, opencoder_session::SessionEvent::AgentSwitch(_))),
        "clear keeps the active agent: no AgentSwitch, got {evs:?}"
    );
}
