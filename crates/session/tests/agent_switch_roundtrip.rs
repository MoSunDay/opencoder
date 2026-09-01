//! Regression: `/plan` -> `/act` is a PURE state switch in both
//! directions. Switching never folds the transcript, never emits
//! `TranscriptReset`, and persists the agent to the store. A switch onto the
//! agent already in charge (`/act` on an act session) is a total no-op: no
//! event, no store write, no transcript side effects.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
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

fn assistant_msg(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
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

fn make_session(
    store: &Arc<dyn Store>,
    id: &str,
    agent: &str,
    client: Arc<dyn ChatStream>,
    workdir: &std::path::Path,
) -> SessionState {
    SessionState::new(
        id,
        resolve_agent(agent).unwrap(),
        config(),
        client,
        workdir.to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created()
}

#[tokio::test]
async fn plan_then_act_round_trip_folds_nothing() {
    let store = mem_store().await;
    seed(&store, "roundtrip", "act").await;

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("explored")]));
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "roundtrip", "act", mock.clone(), dir.path());

    // Some real history a fold would collapse if switches folded.
    session.record(Message::user("u1", "look around")).await;
    session
        .record(assistant_msg("a1", "found three modules"))
        .await;
    let before: Vec<String> = session.messages.iter().map(|m| m.id.clone()).collect();

    // /plan: agent switches, transcript untouched.
    let mut evs = Vec::new();
    run(&mut session, "/plan".into(), |ev| evs.push(ev))
        .await
        .unwrap();
    assert_eq!(session.agent.name, "plan");
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "plan")),
        "AgentSwitch(plan) emitted, got: {evs:?}"
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, SessionEvent::TranscriptReset(_))),
        "a pure switch must NOT emit TranscriptReset, got: {evs:?}"
    );

    // /act: switch back. Same contract — no fold, no reset.
    let mut evs = Vec::new();
    run(&mut session, "/act".into(), |ev| evs.push(ev))
        .await
        .unwrap();
    assert_eq!(session.agent.name, "act");
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "act")),
        "AgentSwitch(act) emitted, got: {evs:?}"
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, SessionEvent::TranscriptReset(_))),
        "returning to act must NOT emit TranscriptReset, got: {evs:?}"
    );

    // Transcript is byte-identical to the pre-switch history: nothing folded.
    let after: Vec<String> = session.messages.iter().map(|m| m.id.clone()).collect();
    assert_eq!(before, after, "switching must not rewrite the transcript");
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.text().contains("found three modules")),
        "history survives the round trip"
    );

    // The final agent is persisted to the store.
    let meta = store.get_session("roundtrip").await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"), "/act persisted");

    // No LLM turn was consumed by either switch.
    assert!(
        mock.requests().is_empty(),
        "control switches never call the LLM"
    );
}

#[tokio::test]
async fn act_while_already_act_is_a_total_noop() {
    let store = mem_store().await;
    seed(&store, "already-act", "act").await;

    let mock = Arc::new(MockChatClient::new());
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "already-act", "act", mock, dir.path());
    session.record(Message::user("u1", "existing work")).await;
    let before: Vec<String> = session.messages.iter().map(|m| m.id.clone()).collect();

    let mut evs = Vec::new();
    run(&mut session, "/act".into(), |ev| evs.push(ev))
        .await
        .unwrap();

    assert!(
        !evs.iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(_))),
        "redundant /act must emit no event, got: {evs:?}"
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, SessionEvent::TranscriptReset(_))),
        "redundant /act must not fold"
    );
    assert_eq!(before.len(), session.messages.len(), "transcript untouched");
    assert_eq!(session.agent.name, "act", "session stays on act");
}
