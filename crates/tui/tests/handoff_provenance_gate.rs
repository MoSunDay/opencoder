//! Worker-side plan-provenance gate for `UiCmd::SwitchAndStart`.
//!
//! `handoff`'s plan extraction is "last assistant message with non-empty
//! text". In act mode with no plan-phase input that is just the previous
//! answer ("task complete") — wrapping it in the plan→act directive would
//! fabricate a plan, wipe the transcript, and persist a `handoff_seq` resume
//! boundary that irrecoverably drops all context.
//!
//! The trigger is a rapid Shift+Tab double-tap (act→plan→act): the sticky
//! UI-side `plan_submitted` flag survives the first tap until the worker's
//! `AgentSwitch("plan")` event round-trips back to the UI, so the second tap
//! can still queue a `SwitchAndStart`. The app-loop fix folds the switch
//! synchronously (see `app_loop_tests::switch_gate_tests`); this file is the
//! defense-in-depth layer: even when a stale `SwitchAndStart` reaches the
//! worker, `plan_input_count > 0` (the session-side source of truth) must
//! hold before the transcript is folded. On gate failure the handoff degrades
//! to a pure switch plus a normal act-mode submission of the captured input
//! text (`worker::handoff_run_prompt`) — an empty capture degrades to an
//! empty turn instead.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, Message, Role};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::{resume, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};
use tokio::sync::mpsc;

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn assistant_with_text(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(opencoder_core::ContentBlock::text(text));
    m
}

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// The bug's exact FIFO sequence: `SwitchAgent("plan")` (which resets the
/// plan-input counter) immediately followed by a stale `SwitchAndStart`
/// fired off the not-yet-folded UI flag. The gate must degrade it to a pure
/// switch: transcript intact, no resume boundary, and the captured composer
/// text submitted as a normal act-mode prompt (`worker::handoff_run_prompt`)
/// — only an empty capture degrades all the way to an empty turn.
#[tokio::test]
async fn stale_double_tap_switch_and_start_preserves_context() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "stale-double-tap".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    // Act-mode history: the "final plan" extraction would fabricate a plan
    // out of "task complete".
    let history = vec![
        Message::user("u1", "refactor the parser module"),
        assistant_with_text("a1", "task complete"),
    ];
    for m in &history {
        store.append_message("stale-double-tap", m).await.unwrap();
    }

    let mock = Arc::new(MockChatClient::new().push_script(vec![text_done("acknowledged")]));
    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let mut sess = SessionState::new(
        "stale-double-tap",
        resolve_agent("act").expect("act agent"),
        Config::default(),
        mock.clone(),
        std::env::temp_dir(),
    )
    .with_store(store.clone());
    sess.messages = history;
    // A prior plan phase armed the (sticky) UI flag; its counter survives in
    // the session until the next plan entry resets it.
    sess.plan_input_count = 2;

    // Tap 1 (worker FIFO): SwitchAgent("plan") resets the plan phase.
    let quit = process_cmd(UiCmd::SwitchAgent("plan".into()), &mut sess, &tx).await;
    assert!(!quit);
    assert_eq!(sess.plan_input_count, 0, "plan entry resets the counter");

    // Tap 2: stale UI flag still fires SwitchAndStart before the
    // AgentSwitch("plan") event folds the flag in the UI.
    let quit = process_cmd(
        UiCmd::SwitchAndStart("act".into(), "draft text".into()),
        &mut sess,
        &tx,
    )
    .await;
    assert!(!quit, "SwitchAndStart must not break the worker loop");

    let mut events: Vec<UiEvent> = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }

    // (1) No transcript collapse: neither reset nor handoff events.
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::TranscriptReset(_)))),
        "gate failure must not emit TranscriptReset"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::PlanHandoff(_)))),
        "gate failure must not emit PlanHandoff"
    );

    // (2) Degrade is visible: a Status hint explains the skipped handoff.
    assert!(
        events.iter().any(|e| matches!(
            e,
            UiEvent::Session(SessionEvent::Status(ref s)) if s.contains("handoff skipped")
        )),
        "gate failure must emit a Status hint, got {:?} events",
        events.len()
    );

    // (3) The UI protocol still completes (the app-loop is awaiting TurnDone).
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UiEvent::TurnDone(ref a) if a == "act")),
        "TurnDone(act) must still be emitted for the degraded turn"
    );

    // (4) In-memory transcript intact verbatim, plus exactly the capture:
    // gate failure degrades to a plain act-mode submission of `extra`
    // (`worker::handoff_run_prompt`), so the input is not swallowed.
    assert_eq!(
        sess.messages.len(),
        4,
        "history preserved verbatim + one submitted user turn + its answer"
    );
    assert!(sess.messages[0]
        .text()
        .contains("refactor the parser module"));
    assert!(sess.messages[1].text().contains("task complete"));
    assert!(
        matches!(sess.messages[2].role, Role::User)
            && sess.messages[2].text().contains("draft text"),
        "captured input-box text must be submitted as a normal user prompt"
    );
    assert!(
        matches!(sess.messages[3].role, Role::Assistant)
            && sess.messages[3].text().contains("acknowledged"),
        "the submitted prompt consumes the scripted mock answer"
    );
    assert_eq!(
        mock.requests().len(),
        1,
        "gate failure degrades to a plain submission: exactly one LLM call — \
         the input must not be swallowed (zero) nor double-sent"
    );

    // (5) No resume boundary persisted; mode switch itself still persisted.
    let meta = store
        .get_session("stale-double-tap")
        .await
        .unwrap()
        .expect("session row exists");
    assert_eq!(meta.agent.as_deref(), Some("act"), "act mode persisted");
    assert!(
        meta.handoff_seq.is_none(),
        "gate failure must not write handoff_seq, got {:?}",
        meta.handoff_seq
    );

    // (6) A resume keeps the full context — nothing was trimmed.
    let resumed = resume(
        store.clone(),
        "stale-double-tap",
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .await
    .expect("resume succeeds");
    assert!(
        resumed
            .messages
            .iter()
            .any(|m| m.text().contains("task complete")),
        "resume must retain the act-mode history, got {} messages",
        resumed.messages.len()
    );
    assert_eq!(resumed.agent.name, "act");
}

/// The gate must not break the legitimate path: a plan phase that recorded
/// real input still folds the transcript and persists the resume boundary.
#[tokio::test]
async fn plan_phase_input_still_hands_off() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "gated-handoff".into(),
            agent: Some("plan".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![text_done("starting")]));
    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let mut sess = SessionState::new(
        "gated-handoff",
        resolve_agent("plan").expect("plan agent"),
        Config::default(),
        mock,
        std::env::temp_dir(),
    )
    .with_store(store.clone());
    sess.messages = vec![
        Message::user("u1", "implement feature X"),
        assistant_with_text("a1", "## Plan\n1. do X"),
    ];
    // Real plan-phase requirement delivered via maybe_tag_plan_prompt.
    sess.plan_input_count = 1;
    // `plan_snapshot` is the phase-bounded truth source (`SessionState::record` captures it when the plan agent answers); seed it to simulate an already-produced planning phase.
    sess.plan_snapshot = Some("## Plan\n1. do X".into());

    let quit = process_cmd(
        UiCmd::SwitchAndStart("act".into(), "".into()),
        &mut sess,
        &tx,
    )
    .await;
    assert!(!quit);

    let mut events: Vec<UiEvent> = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::TranscriptReset(_)))),
        "legitimate handoff must still emit TranscriptReset"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            UiEvent::Session(SessionEvent::PlanHandoff(ref p)) if p.contains("## Plan")
        )),
        "legitimate handoff must still emit PlanHandoff with the plan"
    );
    let meta = store
        .get_session("gated-handoff")
        .await
        .unwrap()
        .expect("session row exists");
    assert!(
        meta.handoff_seq.is_some(),
        "legitimate handoff must persist the resume boundary"
    );
}
