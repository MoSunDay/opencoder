//! Integration tests for the `/act_clear_context` preservation contract after
//! plan-mode toggles, plus the seed/sentinel split and resume rebuild.
//!
//! * Real plan provenance → plan→act handoff path (`SessionEvent::PlanHandoff`
//!   + directive execution turn).
//! * No provenance but a last non-empty assistant reply → ONE synthetic seed
//!   message (neutral prefix + last-say), persisted as
//!   `handoff_plan = "<<OPENCODER_CLEAR_SEED>>" + last_say`; the run CONTINUES
//!   (one LLM call; raw marker never reaches the model).
//! * No assistant text at all → blank sentinel path, no LLM call.
//! * `/plan` resets ONLY `plan_input_count`; `plan_snapshot` survives switches
//!   and is retired by a newly recorded plan requirement.
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

/// `/act` → `/plan` toggles reset only `plan_input_count`; the phase snapshot
/// survives, so a subsequent `/act_clear_context` still takes the genuine
/// plan→act handoff path: PlanHandoff emitted, plan directive executed once.
#[tokio::test]
async fn toggle_twice_then_clear_keeps_and_executes_plan() {
    let store = mem_store().await;
    seed(&store, "toggle-clear-plan", "plan").await;

    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("done implementing")]));
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(
        &store,
        "toggle-clear-plan",
        "plan",
        mock.clone(),
        dir.path(),
    );

    session
        .record(Message::user("u1", "plan implement Z"))
        .await;
    session
        .record(assistant_msg("a1", "## Plan: implement Z"))
        .await;
    assert_eq!(
        session.plan_snapshot.as_deref(),
        Some("## Plan: implement Z"),
        "record under the plan agent captures the snapshot"
    );

    // Toggle away and back: only the counter resets, the snapshot survives.
    run(&mut session, "/act".into(), |_| {}).await.unwrap();
    run(&mut session, "/plan".into(), |_| {}).await.unwrap();
    assert_eq!(
        session.plan_snapshot.as_deref(),
        Some("## Plan: implement Z"),
        "plan_snapshot must survive the /act -> /plan toggle"
    );
    assert_eq!(session.plan_input_count, 0, "counter reset by /plan");

    let mut evs = Vec::new();
    run(&mut session, "/act_clear_context".into(), |ev| evs.push(ev))
        .await
        .unwrap();

    // Plan directive path: exactly one execution turn carrying the plan.
    assert_eq!(mock.call_count(), 1, "one LLM call to execute the plan");
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let body = requests[0].to_body().to_string();
    assert!(
        body.contains("## Plan: implement Z"),
        "plan text must reach the model: {body}"
    );
    assert!(
        body.contains("Execute it now"),
        "plan directive prefix must reach the model: {body}"
    );

    assert_eq!(
        session.handoff_plan.as_deref(),
        Some("## Plan: implement Z"),
        "handoff_plan stores the raw plan text"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::PlanHandoff(p) if p == "## Plan: implement Z")),
        "PlanHandoff emitted carrying the plan"
    );
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.role == Role::Assistant && m.text().contains("done implementing")),
        "execution reply recorded as an assistant turn"
    );
}

/// ACT history with no plan provenance: clear seeds continuity from the last
/// assistant reply, persists the seed marker, and CONTINUES the run (one LLM
/// call whose body carries the last say, never the raw marker nor the plan
/// directive).
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
    assert_eq!(session.plan_input_count, 0);
    assert!(session.plan_snapshot.is_none());

    let mut evs = Vec::new();
    run(&mut session, "/act_clear_context".into(), |ev| evs.push(ev))
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
        "seed must not use the plan directive: {seed_text}"
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
        "plan directive must not be fabricated: {body}"
    );

    assert!(
        !evs.iter()
            .any(|e| matches!(e, SessionEvent::PlanHandoff(_))),
        "seed path emits no PlanHandoff"
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

/// A newly recorded plan requirement retires the stale phase snapshot (both in
/// memory and in the store mirror) even when the requirement's turn failed
/// before any assistant output landed.
#[tokio::test]
async fn failed_new_requirement_retires_stale_snapshot() {
    let store = mem_store().await;
    seed(&store, "retire-snapshot", "plan").await;

    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(
        &store,
        "retire-snapshot",
        "plan",
        Arc::new(MockChatClient::new()),
        dir.path(),
    );

    session.record(Message::user("u1", "plan W")).await;
    session.record(assistant_msg("a1", "old plan v1")).await;
    assert_eq!(session.plan_snapshot.as_deref(), Some("old plan v1"));

    // Toggle act -> plan: the snapshot must survive both switches.
    let switch = |n: &str| opencoder_session::control_cmd::ControlCmd::SwitchAgent(n.to_string());
    opencoder_session::control_cmd::apply(&mut session, &switch("act"), &mut |_| {})
        .await
        .unwrap();
    opencoder_session::control_cmd::apply(&mut session, &switch("plan"), &mut |_| {})
        .await
        .unwrap();
    assert!(
        session.plan_snapshot.is_some(),
        "snapshot survives the act->plan switches"
    );
    assert_eq!(session.plan_input_count, 0, "counter reset by the switch");

    // New plan requirement whose turn fails: no assistant ever recorded.
    let mut req = "plan feature W".to_string();
    session.maybe_tag_plan_prompt(&mut req);
    session.record(Message::user("u2", "plan feature W")).await;
    session.persist_plan_phase().await;

    assert_eq!(session.plan_snapshot, None, "stale snapshot retired");
    let meta = store.get_session("retire-snapshot").await.unwrap().unwrap();
    assert_eq!(meta.plan_snapshot, None, "store mirror retired too");
}

/// Compound `/act_clear_context <request>` on the seed path: the request is
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
        "/act_clear_context retry the build".into(),
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
        !body.contains("/act_clear_context"),
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

    run(&mut session, "/act_clear_context".into(), |_| {})
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
/// prefix + last say), never the raw marker nor the plan directive.
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
        "no plan directive: {text}"
    );
}
