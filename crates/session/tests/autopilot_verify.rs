//! Shadow-VERIFY isolation tests for the autopilot loop: the small-model
//! judge runs against a throwaway transcript snapshot, retries/clamps on
//! unparseable answers, and never pollutes the live session. Full-loop
//! drives live in `autopilot.rs`; the one-shot review pass in
//! `autopilot_review.rs`.

use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, AutoPilotConfig, Config, Message};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::autopilot::{
    drive, verify, ApOutcome, ApState, VerifyFailure, VerifyVerdict,
};
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

fn autopilot_config(max_iterations: u32, verify_retries: u32) -> Config {
    Config {
        autopilot: AutoPilotConfig {
            mode: opencoder_core::ApMode::Ap,
            max_iterations,
            verify_retries,
            ..AutoPilotConfig::default()
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

#[tokio::test]
async fn verify_yes_means_complete_and_does_not_pollute_transcript() {
    let mock = Arc::new(MockChatClient::new().push_script(vec![completed("yes", vec![])]))
        as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));
    session.record(Message::user("u1", "do the thing")).await;
    let state = ApState::new("do the thing".into());
    let before = session.messages.len();

    let verdict = verify(&session, &state, 3).await.unwrap();
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
    let verdict = verify(&session, &state, 3).await.unwrap();
    assert_eq!(verdict, VerifyVerdict::MoreWork);
}

#[tokio::test]
async fn verify_garbage_retries_then_unparseable() {
    // 3 garbage answers + verify_retries=3 -> Err(Unparseable) naming the
    // attempt count (each retry consumed).
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
    assert_eq!(
        verdict,
        Err(VerifyFailure::Unparseable { attempts: 3 }),
        "exhausted unparseable budget must report the cause + attempts"
    );
    assert_eq!(session.messages.len(), before, "transcript untouched");
}

#[tokio::test]
async fn verify_transport_errors_report_unreachable() {
    // Every judge call dies at the transport layer -> Err(Unreachable)
    // carrying the LAST error verbatim — distinguishable from the
    // unparseable-judge exhaustion above.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Error("conn refused".into())])
            .push_script(vec![LlmEvent::Error("429 rate limited".into())]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 2));
    session.record(Message::user("u1", "do the thing")).await;
    let state = ApState::new("do the thing".into());
    let before = session.messages.len();

    let verdict = verify(&session, &state, 2).await;
    assert_eq!(
        verdict,
        Err(VerifyFailure::Unreachable {
            attempts: 2,
            last_error: "429 rate limited".into(),
        }),
        "transport exhaustion must report unreachable + the last error"
    );
    assert_eq!(session.messages.len(), before, "transcript untouched");
}

#[tokio::test]
async fn verify_qualified_yes_is_unparseable_not_complete() {
    // Strict single-token parsing: "Yes, more work" is NOT a Complete — it
    // burns a retry; here it exhausts the budget as Unparseable.
    let mock = Arc::new(
        MockChatClient::new().push_script(vec![completed("Yes, more work needed", vec![])]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut session) = make_session(mock, autopilot_config(10, 1));
    session.record(Message::user("u1", "do the thing")).await;
    let state = ApState::new("do the thing".into());
    let verdict = verify(&session, &state, 1).await;
    assert_eq!(verdict, Err(VerifyFailure::Unparseable { attempts: 1 }));
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
    let verdict = verify(&session, &state, 3).await.unwrap();
    assert_eq!(verdict, VerifyVerdict::Complete);
}

// ── drive(): full loop outcomes ───────────────────────────────────────────

#[tokio::test]
async fn verify_retries_zero_is_clamped_to_one() {
    // verify_retries=0 would never judge; drive clamps to 1 so a single
    // unparseable answer still aborts (rather than silently never calling
    // the judge and aborting with an empty cause immediately).
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
    let verdict = verify(&session, &state, 3).await.unwrap();
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
