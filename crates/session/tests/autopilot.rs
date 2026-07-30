//! Integration tests for the autopilot loop: shadow VERIFY isolation, full
//! PLAN -> ACT -> VERIFY drives, abort/max-iteration outcomes, and the
//! enabled=false gate.

use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, AutoPilotConfig, Config, Message};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::autopilot::{drive, verify, ApOutcome, ApPhase, ApState, VerifyVerdict};
use opencoder_session::runner::run_with_registry;
use opencoder_session::tools::registry;
use opencoder_session::{SessionEvent, SessionState};

/// A completed turn with optional tool calls (empty tools = idle/Done).
fn completed(text: &str, tool_calls: Vec<CompletedToolCall>) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls,
        usage: Some(Usage::default()),
    }
}

/// A bash tool-call turn carrying `n` (used for doom-loop + tool execution).
fn bash_turn(n: u32) -> LlmEvent {
    LlmEvent::Completed {
        text: format!("turn-{n}"),
        tool_calls: vec![CompletedToolCall {
            id: format!("tu{n}"),
            name: "bash".into(),
            input: serde_json::json!({"command": "true"}),
        }],
        usage: Some(Usage::default()),
    }
}

fn autopilot_config(max_iterations: u32, verify_retries: u32) -> Config {
    Config {
        model: "m/g".into(),
        autopilot: AutoPilotConfig {
            enabled: true,
            max_iterations,
            verify_retries,
        },
        ..Config::default()
    }
}

fn make_session(mock: Arc<dyn ChatStream>, config: Config) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let s = SessionState::new("ap-sess", agent, config, mock, dir.path().to_path_buf());
    (dir, s)
}

fn collector() -> (Arc<Mutex<Vec<SessionEvent>>>, impl FnMut(SessionEvent)) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let clone = buf.clone();
    let f = move |ev: SessionEvent| clone.lock().unwrap().push(ev);
    (buf, f)
}

fn phase_label(phase: &ApPhase) -> &'static str {
    match phase {
        ApPhase::Plan => "plan",
        ApPhase::Act => "act",
        ApPhase::Verify => "verify",
    }
}

// ── verify(): shadow one-shot isolation ───────────────────────────────────

#[tokio::test]
async fn verify_yes_means_more_work_and_does_not_pollute_transcript() {
    let mock = Arc::new(MockChatClient::new().push_script(vec![completed("yes", vec![])]))
        as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session.record(Message::user("u1", "do the thing")).await;
    let state = ApState::new("do the thing".into());
    let before = session.messages.len();

    let verdict = verify(&session, &state, 3).await;
    assert_eq!(verdict, VerifyVerdict::MoreWork);
    assert_eq!(
        session.messages.len(),
        before,
        "VERIFY must never append to session.messages"
    );
}

#[tokio::test]
async fn verify_no_means_complete() {
    let mock = Arc::new(MockChatClient::new().push_script(vec![completed("no", vec![])]))
        as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session.record(Message::user("u1", "do the thing")).await;
    let state = ApState::new("do the thing".into());
    let verdict = verify(&session, &state, 3).await;
    assert_eq!(verdict, VerifyVerdict::Complete);
}

#[tokio::test]
async fn verify_garbage_retries_then_malformed() {
    // 3 garbage answers + verify_retries=3 -> Malformed (each retry consumed).
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("maybe", vec![])])
            .push_script(vec![completed("not sure", vec![])])
            .push_script(vec![completed("??? lol", vec![])]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session.record(Message::user("u1", "do the thing")).await;
    let state = ApState::new("do the thing".into());
    let before = session.messages.len();

    let verdict = verify(&session, &state, 3).await;
    assert_eq!(verdict, VerifyVerdict::Malformed);
    assert_eq!(session.messages.len(), before, "transcript untouched");
}

#[tokio::test]
async fn verify_retries_until_a_parseable_answer() {
    // 2 garbage then "no" -> Complete (retries recover, not malformed).
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("hmm", vec![])])
            .push_script(vec![completed("no", vec![])]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session.record(Message::user("u1", "do the thing")).await;
    let state = ApState::new("do the thing".into());
    let verdict = verify(&session, &state, 3).await;
    assert_eq!(verdict, VerifyVerdict::Complete);
}

// ── drive(): full loop outcomes ───────────────────────────────────────────

#[tokio::test]
async fn drive_completes_when_verify_says_no() {
    // iteration 0: plan, act, verify(yes=MoreWork)
    // iteration 1: plan, act, verify(no=Complete)
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("plan-0", vec![])])
            .push_script(vec![completed("act-0", vec![])])
            .push_script(vec![completed("yes", vec![])])
            .push_script(vec![completed("plan-1", vec![])])
            .push_script(vec![completed("act-1", vec![])])
            .push_script(vec![completed("no", vec![])]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session
        .record(Message::user("u1", "implement feature X"))
        .await;

    let reg = registry();
    let (_buf, mut on_event) = collector();
    let outcome = drive(&mut session, &reg, &mut on_event).await.unwrap();
    assert_eq!(outcome, ApOutcome::Complete);
}

#[tokio::test]
async fn drive_emits_autopilot_phase_events() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("plan-0", vec![])])
            .push_script(vec![completed("act-0", vec![])])
            .push_script(vec![completed("no", vec![])]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session
        .record(Message::user("u1", "implement feature X"))
        .await;

    let reg = registry();
    let (buf, mut on_event) = collector();
    drive(&mut session, &reg, &mut on_event).await.unwrap();

    let phases: Vec<&'static str> = buf
        .lock()
        .unwrap()
        .iter()
        .filter_map(|ev| match ev {
            SessionEvent::AutoPilot { phase, .. } => Some(phase_label(phase)),
            _ => None,
        })
        .collect();
    // First (only) iteration must cycle Plan -> Act -> Verify.
    assert_eq!(
        phases,
        vec!["plan", "act", "verify"],
        "expected one full phase cycle before Complete"
    );
}

#[tokio::test]
async fn drive_aborts_when_verify_keeps_malformed() {
    // plan, act, then verify retries 3x garbage -> Malformed -> Aborted.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("plan-0", vec![])])
            .push_script(vec![completed("act-0", vec![])])
            .push_script(vec![completed("garbage", vec![])])
            .push_script(vec![completed("??", vec![])])
            .push_script(vec![completed("xyz", vec![])]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session
        .record(Message::user("u1", "implement feature X"))
        .await;

    let reg = registry();
    let (_buf, mut on_event) = collector();
    let outcome = drive(&mut session, &reg, &mut on_event).await.unwrap();
    match outcome {
        ApOutcome::Aborted(_) => {}
        other => panic!("expected Aborted, got {other:?}"),
    }
}

#[tokio::test]
async fn drive_max_iterations_one_yields_max_iterations() {
    // max=1: iteration 0 plan, act, verify(yes=MoreWork) -> at cap -> MaxIterations.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("plan-0", vec![])])
            .push_script(vec![completed("act-0", vec![])])
            .push_script(vec![completed("yes", vec![])]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(1, 3));
    session
        .record(Message::user("u1", "implement feature X"))
        .await;

    let reg = registry();
    let (_buf, mut on_event) = collector();
    let outcome = drive(&mut session, &reg, &mut on_event).await.unwrap();
    assert_eq!(outcome, ApOutcome::MaxIterations);
}

// ── gating: enabled=false never starts the loop ──────────────────────────

#[tokio::test]
async fn autopilot_disabled_never_invokes_drive() {
    // With autopilot off, run_with_registry runs the initial task only: a
    // single LLM call that idles (Done). No plan/act/verify calls follow.
    let mock = Arc::new(MockChatClient::new().push_script(vec![completed("done", vec![])]));
    let cfg = Config {
        model: "m/g".into(),
        ..Config::default()
    };
    assert!(!cfg.autopilot.enabled, "autopilot is off by default");
    let (_dir, mut session) = make_session(mock.clone() as Arc<dyn ChatStream>, cfg);

    let reg = registry();
    let (_buf, mut on_event) = collector();
    run_with_registry(&mut session, "kickoff".into(), vec![], &reg, &mut on_event)
        .await
        .unwrap();

    assert_eq!(
        mock.call_count(),
        1,
        "disabled autopilot must not start the loop"
    );
}

#[tokio::test]
async fn autopilot_enabled_via_run_with_registry_completes() {
    // End-to-end: initial task (1 call) + drive iteration 0 (plan/act/verify=no).
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("initial", vec![])])
            .push_script(vec![completed("plan-0", vec![])])
            .push_script(vec![completed("act-0", vec![])])
            .push_script(vec![completed("no", vec![])]),
    );
    let (_dir, mut session) =
        make_session(mock.clone() as Arc<dyn ChatStream>, autopilot_config(10, 3));

    let reg = registry();
    let (_buf, mut on_event) = collector();
    run_with_registry(&mut session, "do it".into(), vec![], &reg, &mut on_event)
        .await
        .unwrap();
    assert_eq!(
        mock.call_count(),
        4,
        "initial task (1) + plan/act/verify (3)"
    );
}

// ── doom-loop guard still terminates a phase run within AP ────────────────

#[tokio::test]
async fn doom_loop_guard_terminates_act_phase() {
    // The act phase's run_loop gets 20 identical bash calls -> doom-loop break
    // (DOOM_THRESHOLD=20). plan (1, idle), act (20 bash -> doom),
    // verify (no -> Complete).
    let mut builder = MockChatClient::new().push_script(vec![completed("plan-0", vec![])]);
    for i in 1..=20u32 {
        builder = builder.push_script(vec![bash_turn(i)]);
    }
    let mock = Arc::new(builder.push_script(vec![completed("no", vec![])])) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(40, 3));
    session
        .record(Message::user("u1", "implement feature X"))
        .await;

    let reg = registry();
    let (_buf, mut on_event) = collector();
    let outcome = drive(&mut session, &reg, &mut on_event).await.unwrap();
    // The doom guard broke the act phase; verify then said Complete.
    assert_eq!(outcome, ApOutcome::Complete);
}

#[tokio::test]
async fn act_phase_handoff_resets_transcript_and_clears_skill() {
    // PLAN produces an assistant message ("plan-0"), ACT resets via handoff
    // (emitting TranscriptReset), then VERIFY says Complete.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("plan-0", vec![])])
            .push_script(vec![completed("act-0", vec![])])
            .push_script(vec![completed("no", vec![])]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session
        .record(Message::user("u1", "implement feature X"))
        .await;

    let reg = registry();
    let (buf, mut on_event) = collector();
    drive(&mut session, &reg, &mut on_event).await.unwrap();

    // ACT phase handoff must have emitted TranscriptReset.
    let has_reset = buf
        .lock()
        .unwrap()
        .iter()
        .any(|ev| matches!(ev, SessionEvent::TranscriptReset(_)));
    assert!(
        has_reset,
        "ACT phase must emit TranscriptReset (plan->act handoff)"
    );

    // Skill must be cleared after the loop completes.
    assert!(
        session.skill_prompt_cloned().is_none(),
        "skill must be cleared after drive completes"
    );

    // The original task message must not survive the handoff reset.
    assert!(
        !session
            .messages
            .iter()
            .any(|m| m.text().contains("implement feature X")),
        "handoff must have removed plan-phase messages from transcript"
    );
}

#[tokio::test]
async fn act_phase_fallback_injects_execute_prompt_when_plan_has_no_text() {
    // PLAN produces an empty assistant message, so handoff returns None and
    // run_act_phase falls back to injecting execute_prompt. VERIFY says Complete.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("", vec![])])
            .push_script(vec![completed("act-0", vec![])])
            .push_script(vec![completed("no", vec![])]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session
        .record(Message::user("u1", "implement feature X"))
        .await;

    let reg = registry();
    let (buf, mut on_event) = collector();
    drive(&mut session, &reg, &mut on_event).await.unwrap();

    // Fallback path must NOT emit TranscriptReset.
    let has_reset = buf
        .lock()
        .unwrap()
        .iter()
        .any(|ev| matches!(ev, SessionEvent::TranscriptReset(_)));
    assert!(!has_reset, "fallback path must not emit TranscriptReset");

    // execute_prompt must have been injected into the transcript.
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.text().contains("Execute the plan you just produced")),
        "fallback must inject execute_prompt into the transcript"
    );

    // The original task message survives the fallback (transcript not reset).
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.text().contains("implement feature X")),
        "fallback must preserve the original plan-phase transcript"
    );

    // Skill must still be cleared after the loop completes.
    assert!(
        session.skill_prompt_cloned().is_none(),
        "skill must be cleared after drive completes (fallback path)"
    );
}
