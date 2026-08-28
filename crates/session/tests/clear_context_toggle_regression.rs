//! Integration tests for the `/clear_context` preservation contract: the
//! seed/sentinel split and resume rebuild.
//!
//! * A last non-empty assistant reply → ONE synthetic seed message (neutral
//!   prefix + last-say), persisted as
//!   `handoff_plan = "<<OPENCODER_CLEAR_SEED>>" + last_say`; the run CONTINUES
//!   (one LLM call; raw marker never reaches the model).
//! * No assistant text at all → blank sentinel path, no LLM call.
//! * Compound `/clear_context <request>` records the rest as a real prompt.
//! * `resume` rebuilds the seed message from the stored marker.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{resume, run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

const SEED_MARKER: &str = "<<OPENCODER_CLEAR_SEED>>";
const BLANK_MARKER: &str = "<<OPENCODER_CLEAR_CONTEXT_MARKER>>";

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

/// A store-attached, created session for `id`/`agent` (caller seeds the store
/// row first with [`seed`]).
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

/// History with any last assistant reply: clear seeds continuity from it,
/// persists the seed marker, and CONTINUES the run (one LLM call whose body
/// carries the last say, never the raw marker nor an execution directive).
#[tokio::test]
async fn act_history_clear_keeps_last_say_seed() {
    let store = mem_store().await;
    seed(&store, "act-seed", "act").await;

    let msgs = vec![
        Message::user("u1", "implement X"),
        assistant_msg("a1", "task done"),
    ];
    store.append_messages("act-seed", &msgs).await.unwrap();

    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("continuing")]));
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "act-seed", "act", mock.clone(), dir.path());
    session.messages = msgs;

    let mut evs = Vec::new();
    run(&mut session, "/clear_context".into(), |ev| evs.push(ev))
        .await
        .unwrap();

    // Transcript collapses to ONE synthetic seed message.
    let seed = &session.messages[0];
    assert_eq!(seed.role, Role::User, "seed is a user message");
    assert!(seed.synthetic, "seed is synthetic");
    let seed_text = seed.text();
    assert!(
        seed_text.contains("task done"),
        "seed preserves the last say: {seed_text}"
    );
    assert!(
        seed_text.contains("prior context"),
        "seed carries the neutral prefix: {seed_text}"
    );
    assert!(
        !seed_text.contains("Execute it now"),
        "seed must not use an execution directive: {seed_text}"
    );

    assert_eq!(
        session.handoff_plan,
        Some(format!("{SEED_MARKER}task done")),
        "handoff_plan stores the seed marker + last say"
    );

    // The run continues: exactly one LLM call.
    assert_eq!(mock.call_count(), 1, "seed path must execute one turn");
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let body = requests[0].to_body().to_string();
    assert!(
        body.contains("task done"),
        "last say reaches the model: {body}"
    );
    assert!(
        !body.contains(SEED_MARKER),
        "raw seed marker must never reach the model: {body}"
    );
    assert!(
        !body.contains("Execute it now"),
        "execution directive must not be fabricated: {body}"
    );

    assert!(
        !evs.iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(_))),
        "clear keeps the agent: no AgentSwitch, got {evs:?}"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::TranscriptReset(_))),
        "seed path emits TranscriptReset"
    );
    assert!(
        session
            .messages
            .last()
            .is_some_and(|m| m.role == Role::Assistant && m.text().contains("continuing")),
        "assistant reply recorded after the seed"
    );
}

/// Compound `/clear_context <request>` on the seed path: the request is
/// recorded as a real user prompt and executed alongside the seed.
#[tokio::test]
async fn compound_clear_with_seed_keeps_rest() {
    let store = mem_store().await;
    seed(&store, "act-seed-compound", "act").await;

    let msgs = vec![
        Message::user("u1", "implement X"),
        assistant_msg("a1", "task done"),
    ];
    store
        .append_messages("act-seed-compound", &msgs)
        .await
        .unwrap();

    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("retrying now")]));
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "act-seed-compound", "act", mock.clone(), dir.path());
    session.messages = msgs;

    run(
        &mut session,
        "/clear_context retry the build".into(),
        |_| {},
    )
    .await
    .unwrap();

    assert!(
        session
            .messages
            .iter()
            .any(|m| m.role == Role::User && !m.synthetic && m.text().contains("retry the build")),
        "trailing request recorded as a real user prompt"
    );
    assert_eq!(mock.call_count(), 1, "one execution turn");
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let body = requests[0].to_body().to_string();
    assert!(
        body.contains("task done") && body.contains("retry the build"),
        "seed + request both reach the model: {body}"
    );
    assert!(
        !body.contains("/clear_context"),
        "raw command must not reach the model: {body}"
    );
}

/// Brand-new session (no assistant text at all): blank sentinel path, the run
/// does NOT call the LLM.
#[tokio::test]
async fn fresh_session_clear_uses_blank_sentinel() {
    let store = mem_store().await;
    seed(&store, "fresh-clear", "act").await;

    let msgs = vec![Message::user("u1", "hello")];
    store.append_messages("fresh-clear", &msgs).await.unwrap();

    let mock: Arc<MockChatClient> = Arc::new(MockChatClient::new());
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "fresh-clear", "act", mock.clone(), dir.path());
    session.messages = msgs;

    run(&mut session, "/clear_context".into(), |_| {})
        .await
        .unwrap();

    assert_eq!(mock.call_count(), 0, "blank sentinel path never executes");
    assert!(mock.requests().is_empty());
    assert_eq!(
        session.messages.len(),
        1,
        "transcript collapses to 1 marker"
    );
    assert!(
        session.messages[0].text().contains("Context cleared"),
        "marker message: {}",
        session.messages[0].text()
    );
    assert_eq!(
        session.handoff_plan.as_deref(),
        Some(BLANK_MARKER),
        "blank sentinel stored for resume"
    );
}

/// Resume of a persisted seed boundary rebuilds the seed message (neutral
/// prefix + last say), never the raw marker nor an execution directive.
#[tokio::test]
async fn resume_rebuilds_seed_message() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "resume-seed".into(),
            agent: Some("act".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            handoff_seq: Some(2),
            handoff_plan: Some(format!("{SEED_MARKER}task done")),
            ..Default::default()
        })
        .await
        .unwrap();
    let msgs = vec![
        Message::user("u1", "implement X"),
        assistant_msg("a1", "task done"),
    ];
    store.append_messages("resume-seed", &msgs).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>;
    let resumed = resume(
        store.clone(),
        "resume-seed",
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();

    assert_eq!(resumed.messages.len(), 1, "seed + nothing else");
    let m = &resumed.messages[0];
    assert_eq!(m.role, Role::User);
    assert!(m.synthetic, "rebuilt seed is synthetic");
    let text = m.text();
    assert!(text.contains("task done"), "last say present: {text}");
    assert!(text.contains("prior context"), "neutral prefix: {text}");
    assert!(!text.contains(SEED_MARKER), "no raw marker: {text}");
    assert!(
        !text.contains("Execute it now"),
        "no execution directive: {text}"
    );
}
