//! Regression test for `/act_clear_context <request>` losing the request when
//! the transcript holds a preserved plan (an assistant message). Previously the
//! preserved-plan branch unconditionally cleared `user_text`, dropping the
//! trailing request. This is a self-contained test file (does not depend on
//! helpers elsewhere) so it stays isolated from unrelated churn.

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

/// Regression: when the transcript holds a finalized plan (plan-mode session
/// with recorded plan inputs), a compound `/act_clear_context <request>` must
/// NOT discard the request. The request is recorded as a real user prompt and
/// executed alongside the plan handoff message.
#[tokio::test]
async fn clear_context_compound_keeps_rest_with_preserved_plan() {
    let store = mem_store().await;
    seed(&store, "clear-compound-plan", "plan").await;

    // A plan-mode session: one recorded plan input produced an assistant
    // plan, so final_plan_text() returns Some and the preserved-plan branch
    // is taken instead of the sentinel branch. (An act-mode session with a
    // plain last assistant text takes the sentinel branch — see
    // `act_mode_clear_context_uses_sentinel_not_fabricated_plan`.)
    let msgs = vec![Message::user("u1", "old question"), {
        let mut m = Message::assistant("a1");
        m.blocks
            .push(ContentBlock::text("I will implement X by..."));
        m
    }];
    store
        .append_messages("clear-compound-plan", &msgs)
        .await
        .unwrap();

    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("fresh reply")]));
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "clear-compound-plan",
        resolve_agent("plan").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = msgs.clone();
    session.plan_input_count = 1;
    // The plan phase's assistant turn was recorded while the plan agent was
    // active, so the phase-bounded snapshot carries it (record() captures it
    // in the real flow; `handoff` reads ONLY the snapshot).
    session.plan_snapshot = Some("I will implement X by...".into());

    run(&mut session, "/act_clear_context review".into(), |_| {})
        .await
        .unwrap();

    // The request must be preserved (regression: it used to be discarded).
    let has_review = session
        .messages
        .iter()
        .any(|m| m.role == Role::User && m.text().contains("review") && !m.synthetic);
    assert!(
        has_review,
        "trailing arg 'review' must be recorded as a real user prompt"
    );

    // The LLM was called exactly once (plan handoff + request execution).
    let requests = mock.requests();
    assert_eq!(
        requests.len(),
        1,
        "one LLM call to execute the preserved plan with the request"
    );

    // Both the preserved plan text and the request reach the model context.
    let body = requests[0].to_body().to_string();
    assert!(
        body.contains("I will implement X by..."),
        "preserved plan must appear in the model context: {body}"
    );
    assert!(
        body.contains("review"),
        "request 'review' must reach the model context: {body}"
    );
    assert!(
        !body.contains("/act_clear_context"),
        "raw command string must not reach the model: {body}"
    );

    // The execution produced an assistant turn with the reply text.
    let has_reply = session
        .messages
        .iter()
        .any(|m| m.role == Role::Assistant && m.text().contains("fresh reply"));
    assert!(has_reply, "execution reply recorded as an assistant turn");
}

/// Contract (updated): `/act_clear_context` in ACT mode (no plan provenance:
/// act agent, `plan_input_count == 0`) must NEVER fully wipe — the last
/// assistant reply ("task done") survives as a NEUTRAL continuity seed. The
/// 653e5bd fabrication guard still holds: the reply is NOT wrapped in the
/// plan→act "Execute it now" directive and no PlanHandoff fires for it; it
/// reaches the model only as prior context. The run continues (the seed is
/// executed), unlike the old blank-sentinel stop.
#[tokio::test]
async fn act_mode_clear_context_seeds_last_say_not_fabricated_plan() {
    let store = mem_store().await;
    seed(&store, "act-no-plan", "act").await;

    // Act-mode history whose last assistant text is a plain completion, not
    // a plan. Pre-653e5bd, `handoff` picked it up and wrapped it in the
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
    assert_eq!(session.plan_input_count, 0, "no plan inputs recorded");

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "/act_clear_context".into(), move |ev| {
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
        "seed must NOT use the plan→act directive prefix"
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

    // The model saw the last say as context — never a fabricated plan
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
        "no fabricated plan directive: {body}"
    );

    let evs = events.lock().unwrap();
    assert!(
        evs.iter()
            .any(|e| matches!(e, opencoder_session::SessionEvent::TranscriptReset(_))),
        "TranscriptReset emitted"
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, opencoder_session::SessionEvent::PlanHandoff(_))),
        "no PlanHandoff for a non-plan seed"
    );
}

/// The gate's second arm: an ACT session that WAS planning earlier in this
/// phase (`plan_input_count > 0` survives a plain `/act` switch) still hands
/// its finalized plan forward — the fix must not over-reach.
#[tokio::test]
async fn act_mode_after_plan_inputs_still_preserves_plan() {
    let store = mem_store().await;
    seed(&store, "act-after-plan", "act").await;

    let msgs = vec![Message::user("u1", "plan the migration"), {
        let mut m = Message::assistant("a1");
        m.blocks
            .push(ContentBlock::text("## Plan\n1. migrate schema"));
        m
    }];
    store
        .append_messages("act-after-plan", &msgs)
        .await
        .unwrap();

    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("executing")]));
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "act-after-plan",
        resolve_agent("act").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = msgs.clone();
    // Simulate: planned in plan mode (inputs recorded + the plan captured in
    // the phase snapshot, which survives a plain `/act` switch exactly like
    // the counter), then `/act` switched without a handoff.
    session.plan_input_count = 2;
    session.plan_snapshot = Some("## Plan\n1. migrate schema".into());

    run(&mut session, "/act_clear_context".into(), |_| {})
        .await
        .unwrap();

    // Plan preserved, not the sentinel: the handoff message carries the plan
    // and one execution turn ran.
    assert_eq!(
        session.handoff_plan.as_deref(),
        Some("## Plan\n1. migrate schema"),
        "plan with recorded plan inputs must be preserved"
    );
    assert_eq!(mock.call_count(), 1, "one execution turn for the plan");
    assert!(
        session.messages[0]
            .text()
            .contains("## Plan\n1. migrate schema"),
        "handoff directive carries the plan text"
    );
}

/// Unit-level (apply()) check of the plan-provenance gate itself: ACT mode,
/// no plan inputs → the last assistant reply survives as a NEUTRAL seed (not
/// a plan directive), so the seed marker is persisted instead of the blank
/// sentinel. Mirrors `act_mode_clear_context_seeds_last_say_not_fabricated_plan`
/// but without the run loop, pinning the gate at the control-command layer.
#[tokio::test]
async fn apply_clear_context_act_mode_seeds_instead_of_fabricating_plan() {
    let mock: Arc<MockChatClient> = Arc::new(MockChatClient::new());
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "act-gate",
        resolve_agent("act").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    );
    session.messages.push(Message::user("u1", "implement X"));
    let mut a = Message::assistant("a1");
    a.blocks.push(ContentBlock::text("task done"));
    session.messages.push(a);
    assert_eq!(session.plan_input_count, 0);

    let mut evs = Vec::new();
    opencoder_session::control_cmd::apply(
        &mut session,
        &opencoder_session::control_cmd::ControlCmd::ClearContext,
        &mut |ev| evs.push(ev),
    )
    .await
    .unwrap();

    assert_eq!(session.messages.len(), 1, "collapses to 1 seed message");
    assert!(
        session.messages[0].text().contains("task done"),
        "seed carries the last say as prior context: {}",
        session.messages[0].text()
    );
    assert!(
        !session.messages[0].text().contains("Execute it now"),
        "seed must NOT wrap the reply in the plan directive prefix"
    );
    assert_eq!(
        session.handoff_plan.as_deref(),
        Some("<<OPENCODER_CLEAR_SEED>>task done")
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, opencoder_session::SessionEvent::PlanHandoff(_))),
        "no PlanHandoff for a fabricated plan"
    );
}

/// Unit-level check of the gate's second arm: ACT mode but plan inputs were
/// recorded earlier in the phase (counter survives a plain `/act` switch) →
/// the finalized plan is still handed forward.
#[tokio::test]
async fn apply_clear_context_act_mode_with_plan_inputs_preserves_plan() {
    let mock: Arc<MockChatClient> = Arc::new(MockChatClient::new());
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "act-gate-plan",
        resolve_agent("act").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    );
    session.messages.push(Message::user("u1", "plan the work"));
    let mut a = Message::assistant("a1");
    a.blocks.push(ContentBlock::text("## Plan\n1. do X"));
    session.messages.push(a);
    session.plan_input_count = 2;
    // Recorded by the plan agent during the phase (survives the `/act` switch).
    session.plan_snapshot = Some("## Plan\n1. do X".into());

    let mut evs = Vec::new();
    opencoder_session::control_cmd::apply(
        &mut session,
        &opencoder_session::control_cmd::ControlCmd::ClearContext,
        &mut |ev| evs.push(ev),
    )
    .await
    .unwrap();

    assert_eq!(
        session.handoff_plan.as_deref(),
        Some("## Plan\n1. do X"),
        "plan with recorded plan inputs must be preserved"
    );
    assert!(
        session.messages[0].text().contains("## Plan\n1. do X"),
        "handoff directive carries the plan text"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, opencoder_session::SessionEvent::PlanHandoff(_))),
        "PlanHandoff emitted for a genuine plan"
    );
}
