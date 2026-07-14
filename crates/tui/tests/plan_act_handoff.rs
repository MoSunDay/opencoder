//! Integration test for plan→act handoff via the TUI worker.
//!
//! Drives the real `process_cmd(UiCmd::SwitchAndStart("act"))` path with a
//! MockChatClient and asserts the three user-facing contracts:
//! 1. A `TranscriptReset` event is emitted (so the UI rebuilds clean).
//! 2. The transcript collapses to a single message carrying the final plan.
//! 3. The act agent's LLM request receives ONLY the handoff message — not the
//!    planning conversation (exploration chatter, the original request).

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::{SessionEvent, SessionState};
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};
use tokio::sync::mpsc;

fn assistant_with_text(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

#[tokio::test]
async fn switch_and_start_clears_transcript_and_feeds_only_plan_to_act() {
    // The act turn returns one completed text turn with no tool calls, so the
    // run loop settles after a single LLM call.
    let mock =
        Arc::new(MockChatClient::new().push_script(vec![text_done("starting implementation now")]));
    let dir = tempfile::tempdir().unwrap();
    let plan_agent = resolve_agent("plan").unwrap();
    let mut session = SessionState::new(
        "handoff-int",
        plan_agent,
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock.clone(),
        dir.path().to_path_buf(),
    );
    // Seed a planning transcript: request + exploration chatter + final plan.
    session.messages = vec![
        Message::user("u1", "implement feature X"),
        assistant_with_text("a1", "let me explore the codebase first..."),
        assistant_with_text("a2", "## Plan\n1. do X\n2. do Y"),
    ];

    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let quit = process_cmd(UiCmd::SwitchAndStart("act".into(), "".into()), &mut session, &tx).await;
    assert!(!quit, "SwitchAndStart must not signal quit");

    let mut events: Vec<UiEvent> = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }

    // (1) AgentSwitch("act") emitted.
    assert!(
        events.iter().any(|e| matches!(
            e,
            UiEvent::Session(SessionEvent::AgentSwitch(ref n)) if n == "act"
        )),
        "AgentSwitch(act) must be emitted"
    );

    // (2) TranscriptReset emitted and carries a single message with the plan.
    let reset_body = events
        .iter()
        .find_map(|e| match e {
            UiEvent::Session(SessionEvent::TranscriptReset(msgs)) => {
                assert_eq!(msgs.len(), 1, "reset transcript must hold one message");
                Some(msgs[0].text())
            }
            _ => None,
        })
        .expect("TranscriptReset must be emitted");
    assert!(
        reset_body.contains("## Plan\n1. do X\n2. do Y"),
        "reset message must carry the final plan, got: {reset_body}"
    );
    assert!(
        !reset_body.contains("explore the codebase first"),
        "earlier planning chatter must be dropped, got: {reset_body}"
    );

    // (2b) PlanHandoff event emitted — carries the raw plan markdown for the
    //      display layer to render as a visible card.
    let handoff_plan = events
        .iter()
        .find_map(|e| match e {
            UiEvent::Session(SessionEvent::PlanHandoff(p)) => Some(p.clone()),
            _ => None,
        })
        .expect("PlanHandoff must be emitted");
    assert!(
        handoff_plan.contains("## Plan\n1. do X\n2. do Y"),
        "PlanHandoff must carry the final plan text, got: {handoff_plan}"
    );
    assert!(
        !handoff_plan.contains("explore the codebase first"),
        "PlanHandoff must not contain planning chatter"
    );

    // (3) The live transcript was rebuilt on the clean slate: the handoff
    //     message is the seed, and the act turn's response is appended after.
    //     Crucially the planning chatter must NOT be present anymore.
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.text().contains("## Plan\n1. do X\n2. do Y")),
        "handoff seed must be in the transcript"
    );
    assert!(
        !session
            .messages
            .iter()
            .any(|m| m.text().contains("explore the codebase first")),
        "planning chatter must be gone from the live transcript"
    );

    // (4) The act agent's LLM request received ONLY system + the handoff user
    //     message — not the planning conversation. Inspect the lowered
    //     request structurally (JSON-escaped newlines make substring matches
    //     unreliable, so read role/content fields directly).
    let requests = mock.requests();
    assert_eq!(requests.len(), 1, "exactly one act LLM call expected");
    let msgs = &requests[0].messages;
    let user_msgs: Vec<&serde_json::Value> = msgs
        .iter()
        .filter(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
        .collect();
    let assistant_msgs = msgs
        .iter()
        .filter(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"))
        .count();
    assert_eq!(
        user_msgs.len(),
        1,
        "only the handoff user message must reach act"
    );
    assert_eq!(
        assistant_msgs, 0,
        "no planning assistant turn may leak to act"
    );

    let content = user_msgs[0]
        .get("content")
        .and_then(|v| v.as_str())
        .expect("user message has content");
    assert!(
        content.contains("## Plan\n1. do X\n2. do Y"),
        "act request must include the plan, got: {content}"
    );
    assert!(
        !content.contains("explore the codebase first"),
        "act request must NOT include planning chatter, got: {content}"
    );
    assert!(
        !content.contains("implement feature X"),
        "act request must NOT include the original planning request, got: {content}"
    );
}

#[tokio::test]
async fn switch_and_start_without_plan_falls_back_gracefully() {
    // No assistant plan text → handoff is a no-op; the agent still switches and
    // the run proceeds on the existing transcript (no TranscriptReset emitted).
    let mock = Arc::new(MockChatClient::new().push_script(vec![text_done("ok")]));
    let dir = tempfile::tempdir().unwrap();
    let plan_agent = resolve_agent("plan").unwrap();
    let mut session = SessionState::new(
        "handoff-noplan",
        plan_agent,
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock,
        dir.path().to_path_buf(),
    );
    session.messages = vec![Message::user("u1", "just talking, no plan yet")];

    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let _ = process_cmd(UiCmd::SwitchAndStart("act".into(), "".into()), &mut session, &tx).await;

    let mut events: Vec<UiEvent> = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::TranscriptReset(_)))),
        "no TranscriptReset when there is no plan to hand off"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::PlanHandoff(_)))),
        "no PlanHandoff when there is no plan to hand off"
    );
    // handoff was a no-op, so the original user message is still the seed
    // (the act turn's response is appended after it).
    assert_eq!(
        session.messages[0].id, "u1",
        "original transcript must be untouched when no plan found"
    );
}

#[tokio::test]
async fn switch_and_start_appends_input_to_plan_handoff() {
    // Plan-mode input left in the box must be appended to the plan in the
    // handoff message and reach the act agent's LLM request.
    let mock =
        Arc::new(MockChatClient::new().push_script(vec![text_done("starting now")]));
    let dir = tempfile::tempdir().unwrap();
    let plan_agent = resolve_agent("plan").unwrap();
    let mut session = SessionState::new(
        "handoff-extra",
        plan_agent,
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock.clone(),
        dir.path().to_path_buf(),
    );
    session.messages = vec![
        Message::user("u1", "implement feature X"),
        assistant_with_text("a1", "## Plan\n1. do X\n2. do Y"),
    ];

    let extra = "Don't forget to add tests for the new module.";
    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let _ = process_cmd(
        UiCmd::SwitchAndStart("act".into(), extra.into()),
        &mut session,
        &tx,
    )
    .await;

    let mut events: Vec<UiEvent> = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }

    // The TranscriptReset message must carry BOTH the plan and the extra input.
    let reset_body = events
        .iter()
        .find_map(|e| match e {
            UiEvent::Session(SessionEvent::TranscriptReset(msgs)) => Some(msgs[0].text()),
            _ => None,
        })
        .expect("TranscriptReset must be emitted");
    assert!(
        reset_body.contains("## Plan\n1. do X\n2. do Y"),
        "reset must carry the plan, got: {reset_body}"
    );
    assert!(
        reset_body.contains(extra),
        "reset must carry the appended input, got: {reset_body}"
    );

    // And the act LLM request must receive it too.
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let user_msgs: Vec<&serde_json::Value> = requests[0]
        .messages
        .iter()
        .filter(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
        .collect();
    assert_eq!(user_msgs.len(), 1, "one handoff user message");
    let content = user_msgs[0]
        .get("content")
        .and_then(|v| v.as_str())
        .expect("content");
    assert!(
        content.contains("## Plan\n1. do X\n2. do Y"),
        "act request must include the plan, got: {content}"
    );
    assert!(
        content.contains(extra),
        "act request must include the appended input, got: {content}"
    );
    // extra must follow the plan.
    let plan_pos = content.find("## Plan").unwrap();
    let extra_pos = content.find(extra).unwrap();
    assert!(plan_pos < extra_pos, "input must follow the plan in the request");
}
