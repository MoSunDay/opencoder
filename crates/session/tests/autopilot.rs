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
use opencoder_store::{LibsqlStore, Store};

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
async fn verify_yes_means_complete_and_does_not_pollute_transcript() {
    let mock = Arc::new(MockChatClient::new().push_script(vec![completed("yes", vec![])]))
        as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session.record(Message::user("u1", "do the thing")).await;
    let state = ApState::new("do the thing".into());
    let before = session.messages.len();

    let verdict = verify(&session, &state, 3).await;
    assert_eq!(verdict, VerifyVerdict::Complete);
    assert_eq!(
        session.messages.len(),
        before,
        "VERIFY must never append to session.messages"
    );
}

#[tokio::test]
async fn verify_no_means_more_work() {
    let mock = Arc::new(MockChatClient::new().push_script(vec![completed("no", vec![])]))
        as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session.record(Message::user("u1", "do the thing")).await;
    let state = ApState::new("do the thing".into());
    let verdict = verify(&session, &state, 3).await;
    assert_eq!(verdict, VerifyVerdict::MoreWork);
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
    // 2 garbage then "yes" -> Complete (retries recover, not malformed).
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("hmm", vec![])])
            .push_script(vec![completed("yes", vec![])]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session.record(Message::user("u1", "do the thing")).await;
    let state = ApState::new("do the thing".into());
    let verdict = verify(&session, &state, 3).await;
    assert_eq!(verdict, VerifyVerdict::Complete);
}

// ── drive(): full loop outcomes ───────────────────────────────────────────

#[tokio::test]
async fn drive_completes_when_verify_says_yes() {
    // iteration 0: plan, act, verify(no=MoreWork)
    // iteration 1: plan, act, verify(yes=Complete)
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("plan-0", vec![])])
            .push_script(vec![completed("act-0", vec![])])
            .push_script(vec![completed("no", vec![])])
            .push_script(vec![completed("plan-1", vec![])])
            .push_script(vec![completed("act-1", vec![])])
            .push_script(vec![completed("yes", vec![])]),
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
            .push_script(vec![completed("yes", vec![])]),
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
    // max=1: iteration 0 plan, act, verify(no=MoreWork) -> at cap -> MaxIterations.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("plan-0", vec![])])
            .push_script(vec![completed("act-0", vec![])])
            .push_script(vec![completed("no", vec![])]),
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
            .push_script(vec![completed("yes", vec![])]),
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
    // verify (yes -> Complete).
    let mut builder = MockChatClient::new().push_script(vec![completed("plan-0", vec![])]);
    for i in 1..=20u32 {
        builder = builder.push_script(vec![bash_turn(i)]);
    }
    let mock = Arc::new(builder.push_script(vec![completed("yes", vec![])])) as Arc<dyn ChatStream>;
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
            .push_script(vec![completed("yes", vec![])]),
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
            .push_script(vec![completed("yes", vec![])]),
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

// ── cancel / config-clamp / snapshot-truncation ────────────────────────────

#[tokio::test]
async fn drive_returns_cancelled_when_session_cancelled_before_loop() {
    // A cancelled token before the loop starts must yield Cancelled (NOT
    // MaxIterations), consume no LLM calls, and still emit a terminal Done.
    let mock = Arc::new(MockChatClient::new());
    let (_dir, mut session) =
        make_session(mock.clone() as Arc<dyn ChatStream>, autopilot_config(10, 3));
    session
        .record(Message::user("u1", "implement feature X"))
        .await;
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    session = session.with_cancel(token);

    let reg = registry();
    let (buf, mut on_event) = collector();
    let outcome = drive(&mut session, &reg, &mut on_event).await.unwrap();
    assert_eq!(outcome, ApOutcome::Cancelled);
    assert_eq!(mock.call_count(), 0, "no phase may run after cancel");
    assert!(
        buf.lock()
            .unwrap()
            .iter()
            .any(|ev| matches!(ev, SessionEvent::Done)),
        "non-Complete terminal paths must emit a final Done"
    );
}

#[tokio::test]
async fn drive_returns_cancelled_when_cancelled_during_act() {
    // Cancel fires while ACT runs a slow bash tool (`sleep 1`): run_loop breaks
    // at the next turn boundary, drive sees the cancel after ACT and returns
    // Cancelled WITHOUT burning a VERIFY call.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("plan-0", vec![])])
            .push_script(vec![LlmEvent::Completed {
                text: "run".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "tuslow".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "sleep 1"}),
                }],
                usage: Some(Usage::default()),
            }]),
    );
    let (_dir, mut session) =
        make_session(mock.clone() as Arc<dyn ChatStream>, autopilot_config(10, 3));
    session
        .record(Message::user("u1", "implement feature X"))
        .await;
    let token = tokio_util::sync::CancellationToken::new();
    session = session.with_cancel(token.clone());
    let cancel_for_wait = token.clone();
    let mock_for_wait = mock.clone();
    // Event-driven, not wall-clock: wait until both the plan and act LLM
    // calls are consumed (the `sleep 1` bash tool is then running inside
    // ACT), then cancel. A fixed sleep could fire too early on a slow CI
    // and land the cancel before ACT, flaking the exact call_count==2 pin.
    tokio::spawn(async move {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
        while mock_for_wait.call_count() < 2 && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        cancel_for_wait.cancel();
    });

    let reg = registry();
    let (buf, mut on_event) = collector();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        drive(&mut session, &reg, &mut on_event),
    )
    .await
    .expect("drive must not hang")
    .unwrap();
    assert_eq!(outcome, ApOutcome::Cancelled);
    assert_eq!(
        mock.call_count(),
        2,
        "plan (1) + act LLM call (1), no VERIFY after cancel"
    );
    assert!(
        buf.lock()
            .unwrap()
            .iter()
            .any(|ev| matches!(ev, SessionEvent::Done)),
        "cancelled terminal path must still emit a final Done"
    );
}

#[tokio::test]
async fn drive_clamps_zero_max_iterations_to_one() {
    // max_iterations=0 is degenerate: drive clamps it to 1, runs exactly one
    // PLAN->ACT->VERIFY cycle, then ends at the cap (VERIFY=no -> MoreWork).
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("plan-0", vec![])])
            .push_script(vec![completed("act-0", vec![])])
            .push_script(vec![completed("no", vec![])]),
    );
    let (_dir, mut session) =
        make_session(mock.clone() as Arc<dyn ChatStream>, autopilot_config(0, 3));
    session
        .record(Message::user("u1", "implement feature X"))
        .await;

    let reg = registry();
    let (buf, mut on_event) = collector();
    let outcome = drive(&mut session, &reg, &mut on_event).await.unwrap();
    assert_eq!(outcome, ApOutcome::MaxIterations);
    assert_eq!(mock.call_count(), 3, "exactly one clamped iteration");
    assert!(
        buf.lock()
            .unwrap()
            .iter()
            .any(|ev| matches!(ev, SessionEvent::Done)),
        "MaxIterations must still emit a final Done"
    );
}

#[tokio::test]
async fn verify_retries_zero_is_clamped_to_one() {
    // verify_retries=0 would never judge; drive clamps to 1 so a single
    // malformed answer still aborts (rather than silently never calling the
    // judge and reporting Malformed immediately).
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("plan-0", vec![])])
            .push_script(vec![completed("act-0", vec![])])
            .push_script(vec![completed("???", vec![])]),
    );
    let (_dir, mut session) =
        make_session(mock.clone() as Arc<dyn ChatStream>, autopilot_config(10, 0));
    session
        .record(Message::user("u1", "implement feature X"))
        .await;

    let reg = registry();
    let (_buf, mut on_event) = collector();
    let outcome = drive(&mut session, &reg, &mut on_event).await.unwrap();
    assert_eq!(
        mock.call_count(),
        3,
        "verify_retries=0 clamps to one judge call (plan+act+judge)"
    );

    match outcome {
        ApOutcome::Aborted(_) => {}
        other => panic!("expected Aborted, got {other:?}"),
    }
}

#[tokio::test]
async fn verify_snapshot_truncates_transcript_to_window() {
    // A transcript far larger than the small-model window must be truncated to
    // the most recent messages that fit `context_limit - reserved`; the goal
    // question (which re-states the goal) is always kept.
    let mock = Arc::new(MockChatClient::new().push_script(vec![completed("yes", vec![])]));
    let mut cfg = autopilot_config(10, 3);
    cfg.context_limit = Some(10_000); // snapshot budget = 10_000 - 2_000 = 8_000 tokens
    let (_dir, mut session) = make_session(mock.clone() as Arc<dyn ChatStream>, cfg);
    for i in 0..20 {
        session
            .record(Message::user(
                format!("u{i}"),
                format!("seed-{i}") + &"x".repeat(2_000),
            ))
            .await;
    }
    let state = ApState::new("implement the thing".into());
    let verdict = verify(&session, &state, 3).await;
    assert_eq!(verdict, VerifyVerdict::Complete, "\"yes\" = achieved");

    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1);
    let msgs = &reqs[0].messages;
    assert!(
        msgs.len() < 20 + 2,
        "transcript must be truncated, got {} messages",
        msgs.len()
    );
    assert!(msgs.len() >= 3, "system + at least one message + question");
    let joined: String = msgs
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("seed-19"), "most recent message kept");
    assert!(!joined.contains("seed-0"), "oldest message truncated");
    assert!(
        joined.contains("Goal: implement the thing"),
        "goal question must always be present"
    );
}

// ── store-boundary accounting across iterations ──────────────────────────

#[tokio::test]
async fn drive_iteration_two_persists_true_store_handoff_boundary() {
    // Two full iterations. The second ACT-phase handoff runs against a
    // transcript whose head is iteration-1's synthetic handoff message (NOT in
    // the store). The persisted handoff_seq and store_message_count() must
    // reflect the TRUE store count, not just messages.len() — otherwise resume
    // trims at the wrong index and re-attaches plan-mode chatter.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("plan-0", vec![])])
            .push_script(vec![completed("act-0", vec![])])
            .push_script(vec![completed("no", vec![])]) // MoreWork -> iteration 1
            .push_script(vec![completed("plan-1", vec![])])
            .push_script(vec![completed("act-1", vec![])])
            .push_script(vec![completed("yes", vec![])]), // Complete
    ) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session.store = Some(Arc::new(LibsqlStore::open_memory().await.unwrap()) as Arc<dyn Store>);
    session
        .record(Message::user("u1", "implement feature X"))
        .await;

    let reg = registry();
    let (_buf, mut on_event) = collector();
    let outcome = drive(&mut session, &reg, &mut on_event).await.unwrap();
    assert_eq!(outcome, ApOutcome::Complete);

    // In-memory head is the iteration-2 synthetic handoff message (absent
    // from the store); every other in-memory message is persisted.
    let store = session.store.clone().expect("store attached");
    let store_msgs = store.load_messages(&session.id).await.unwrap();
    assert_eq!(
        session.store_message_count(),
        store_msgs.len(),
        "store_message_count must equal the true store message count"
    );
    let hs = session.handoff_seq.expect("handoff_seq persisted") as usize;
    assert_eq!(
        hs + session.messages.len() - 1,
        store_msgs.len(),
        "handoff_seq must be the true store count at the ACT boundary"
    );
}

#[tokio::test]
async fn drive_phase_error_clears_skill_and_emits_done() {
    // Scripts cover iteration 0 (plan/act/verify=MoreWork); iteration 1's
    // PLAN phase hits the exhausted mock -> run_loop error. The drive must
    // still run the terminal bookkeeping (skill cleared + Done event) before
    // propagating the error, so the next user turn doesn't inherit the
    // review skill and the UI gets a uniform end-of-autopilot marker.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("plan-0", vec![])])
            .push_script(vec![completed("act-0", vec![])])
            .push_script(vec![completed("no", vec![])]), // MoreWork -> iteration 1
    ) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session
        .record(Message::user("u1", "implement feature X"))
        .await;

    let reg = registry();
    let (buf, mut on_event) = collector();
    let res = drive(&mut session, &reg, &mut on_event).await;
    assert!(res.is_err(), "phase error must propagate to the caller");
    assert!(
        session.skill_prompt_cloned().is_none(),
        "review skill must be cleared even on a phase error"
    );
    assert!(
        buf.lock()
            .unwrap()
            .iter()
            .any(|ev| matches!(ev, SessionEvent::Done)),
        "terminal Done event must be emitted on a phase error"
    );
}
