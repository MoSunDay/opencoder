//! Regression tests for the RE-clear path of `/clear_context`
//! (`/act_clear_context`): a second clear that fires while the transcript
//! holds ONLY synthetic messages — no non-synthetic assistant text left for
//! `handoff::newest_work_text` / `last_assistant_text` to extract — must NOT
//! overwrite the already-preserved boundary (`handoff_plan`) with the blank
//! sentinel. The blank sentinel overwrite silently dropped the preserved
//! plan from both the UI (Plan card rebuild filters the sentinel) and the
//! model, so a second Shift+Tab confirm (or a resume-then-clear) wiped the
//! plan card entirely.
//!
//! Contract pinned here:
//! * directive boundary re-clear -> directive display preserved, marker
//!   rebuilt as `handoff_message(prev)`, run still executes the plan;
//! * seed boundary re-clear -> seed preserved as `seed_message(prev text)`;
//! * genuinely blank boundary (sentinel / none) -> fresh-start sentinel, no
//!   LLM call (unchanged);
//! * resume rebuilds the exact same flavour the in-memory fold produced.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{
    apply_control_cmd, is_clear_context_handoff, resume, run, ControlCmd, SessionEvent,
    SessionState,
};
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

async fn seed_row(store: &Arc<dyn Store>, id: &str, agent: &str) {
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

fn assistant_say(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

fn make_session(
    store: &Arc<dyn Store>,
    id: &str,
    agent: &str,
    client: Arc<dyn ChatStream>,
) -> SessionState {
    let dir = tempfile::tempdir().unwrap();
    SessionState::new(
        id,
        resolve_agent(agent).unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created()
}

/// Apply a bare control command through the same seam the runner uses for
/// the idle short-circuit, queue drain, and steer intercept. Two back-to-back
/// applies with no LLM turn in between reproduce the re-fire window (the
/// second Shift+Tab confirm lands before the act turn produced any text).
async fn apply_clear(session: &mut SessionState, evs: &mut Vec<SessionEvent>) {
    apply_control_cmd(session, &ControlCmd::ClearContext, &mut |ev| evs.push(ev))
        .await
        .unwrap();
}

/// Drain-mode entry (`run` with an empty prompt) executes a preserved
/// boundary: the last message is a synthetic user marker and the persisted
/// boundary is non-sentinel. A sentinel boundary must NOT reach the model.
async fn drain_turn(session: &mut SessionState) {
    run(session, String::new(), |_| {}).await.unwrap();
}

#[tokio::test]
async fn plan_directive_survives_reclear_and_still_executes() {
    let store = mem_store().await;
    seed_row(&store, "reclear-plan", "plan").await;
    let msgs = vec![
        Message::user("u1", "plan the refactor"),
        assistant_say("a1", "the plan brief"),
    ];
    store.append_messages("reclear-plan", &msgs).await.unwrap();

    let mock: Arc<MockChatClient> = Arc::new(
        MockChatClient::new().push_script(vec![done_turn("executing the preserved plan")]),
    );
    let mut session = make_session(&store, "reclear-plan", "plan", mock.clone());
    session.messages = msgs;

    let mut evs = Vec::new();
    // First clear: plan -> act execution handoff, directive preserved.
    apply_clear(&mut session, &mut evs).await;
    assert_eq!(session.agent.name, "act", "handoff converges to act");
    assert_eq!(
        session.handoff_plan.as_deref(),
        Some("the plan brief"),
        "first clear preserves the directive display"
    );

    // Second clear BEFORE the act turn produced text: transcript holds only
    // the synthetic directive marker, so extraction has nothing to find.
    // The bug overwrote the boundary with the blank sentinel here.
    apply_clear(&mut session, &mut evs).await;
    assert_eq!(
        session.handoff_plan.as_deref(),
        Some("the plan brief"),
        "re-clear must keep the preserved directive, not the sentinel"
    );
    assert!(!is_clear_context_handoff(
        session.handoff_plan.as_deref().unwrap_or("")
    ));
    assert_eq!(session.messages.len(), 1, "transcript stays folded");
    let marker = &session.messages[0];
    assert_eq!(marker.role, Role::User);
    assert!(marker.synthetic);
    let body = marker.text();
    assert!(
        body.contains("Execute it now") && body.contains("the plan brief"),
        "rebuilt marker is the handoff directive: {body}"
    );
    assert!(
        !body.contains(BLANK_MARKER),
        "sentinel never reaches the LLM"
    );
    let meta = store.get_session("reclear-plan").await.unwrap().unwrap();
    assert_eq!(
        meta.handoff_plan.as_deref(),
        Some("the plan brief"),
        "persisted boundary keeps the plan so the UI Plan card survives"
    );

    // The preserved directive still drives an LLM turn (drain mode executes
    // the folded marker because the boundary is non-sentinel).
    drain_turn(&mut session).await;
    assert_eq!(
        mock.call_count(),
        1,
        "non-sentinel boundary must execute, not go idle"
    );
    assert!(session
        .messages
        .last()
        .unwrap()
        .text()
        .contains("executing the preserved plan"));

    // Resume rebuilds the exact same directive flavour.
    let resumed = resume(
        store.clone(),
        "reclear-plan",
        config(),
        Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
        session.working_dir.clone(),
    )
    .await
    .unwrap();
    assert_eq!(resumed.agent.name, "act");
    let head = &resumed.messages[0];
    assert!(head.synthetic);
    assert!(
        head.text().contains("Execute it now") && head.text().contains("the plan brief"),
        "resume rebuilds the directive marker: {}",
        head.text()
    );
}

#[tokio::test]
async fn seed_boundary_survives_reclear_and_still_executes() {
    let store = mem_store().await;
    seed_row(&store, "reclear-seed", "act").await;
    let msgs = vec![
        Message::user("u1", "implement X"),
        assistant_say("a1", "task done"),
    ];
    store.append_messages("reclear-seed", &msgs).await.unwrap();

    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("continuing")]));
    let mut session = make_session(&store, "reclear-seed", "act", mock.clone());
    session.messages = msgs;

    let mut evs = Vec::new();
    apply_clear(&mut session, &mut evs).await;
    assert_eq!(
        session.handoff_plan.as_deref(),
        Some(format!("{SEED_MARKER}task done").as_str()),
        "first clear preserves the last say as a seed"
    );

    // Re-clear with only the synthetic seed marker in the transcript.
    apply_clear(&mut session, &mut evs).await;
    let boundary = session.handoff_plan.clone().unwrap_or_default();
    assert!(
        boundary.starts_with(SEED_MARKER) && boundary.contains("task done"),
        "seed boundary survives the re-clear: {boundary}"
    );
    assert!(!is_clear_context_handoff(&boundary));
    assert_eq!(session.messages.len(), 1);
    let marker = &session.messages[0];
    assert!(marker.synthetic);
    let body = marker.text();
    assert!(
        body.contains("task done") && body.contains("prior context"),
        "rebuilt seed marker keeps the preserved reply + neutral prefix: {body}"
    );
    assert!(
        !body.contains(SEED_MARKER),
        "raw seed marker never reaches the LLM"
    );
    let meta = store.get_session("reclear-seed").await.unwrap().unwrap();
    assert_eq!(meta.handoff_plan.as_deref(), Some(boundary.as_str()));

    drain_turn(&mut session).await;
    assert_eq!(mock.call_count(), 1, "seed re-clear still executes a turn");

    // Resume rebuilds the exact same seed flavour.
    let resumed = resume(
        store.clone(),
        "reclear-seed",
        config(),
        Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
        session.working_dir.clone(),
    )
    .await
    .unwrap();
    let head = &resumed.messages[0];
    assert!(head.synthetic);
    assert!(
        head.text().contains("task done") && head.text().contains("prior context"),
        "resume rebuilds the seed message: {}",
        head.text()
    );
}

#[tokio::test]
async fn blank_boundary_reclear_stays_blank_without_llm() {
    let store = mem_store().await;
    seed_row(&store, "reclear-blank", "act").await;

    let mock: Arc<MockChatClient> = Arc::new(MockChatClient::new());
    let mut session = make_session(&store, "reclear-blank", "act", mock.clone());
    session.messages = Vec::new();

    let mut evs = Vec::new();
    apply_clear(&mut session, &mut evs).await;
    assert_eq!(session.handoff_plan.as_deref(), Some(BLANK_MARKER));

    // Re-clear with nothing preserved anywhere: sentinel stays, fresh-start
    // marker rebuilt, and the drain still stops without an LLM call.
    apply_clear(&mut session, &mut evs).await;
    assert_eq!(
        session.handoff_plan.as_deref(),
        Some(BLANK_MARKER),
        "genuinely blank boundary stays the fresh-start sentinel"
    );
    let marker = &session.messages[0];
    assert!(marker.synthetic);
    assert!(
        marker
            .text()
            .contains("[Context cleared - starting fresh.]"),
        "fresh-start marker rebuilt: {}",
        marker.text()
    );

    drain_turn(&mut session).await;
    assert_eq!(
        mock.call_count(),
        0,
        "blank sentinel must not trigger an LLM call"
    );
}
