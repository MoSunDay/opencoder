//! Integration tests for the direction-aware running gate on mode switches.
//!
//! Mirrors the `plan_act_handoff.rs` harness (MockChatClient + the real
//! `process_cmd`), driving the SAME single-threaded FIFO worker loop the TUI
//! spawns (`while let Some(cmd) = cmd_rx.recv()`). Contracts under test:
//!
//! 1. plan→act while a plan turn is in flight (a hanging LLM call): the
//!    app-loop gate intercepts the BackTab — nothing is applied, no
//!    TranscriptReset / PlanHandoff / AgentSwitch("act"), no act turn. Once
//!    the turn settles (TurnDone = idle boundary), the re-pressed BackTab
//!    performs the full plan→act handoff.
//! 2. act→plan while a turn is in flight: a pure `UiCmd::SwitchAgent("plan")`
//!    is queued and consumed strictly at the turn boundary — the in-flight
//!    turn finishes under act (TurnDone("act") precedes AgentSwitch), and no
//!    second turn is started.

use std::sync::Arc;
use std::time::Duration;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::{SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
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

/// Spawn the same single-threaded FIFO worker loop the real TUI runs. The
/// returned task yields the final `SessionState` after `UiCmd::Quit`.
async fn spawn_worker(
    sess: SessionState,
) -> (
    mpsc::Sender<UiCmd>,
    mpsc::Receiver<UiEvent>,
    tokio::task::JoinHandle<SessionState>,
) {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (evt_tx, evt_rx) = mpsc::channel::<UiEvent>(512);
    let handle = tokio::spawn(async move {
        let mut sess = sess;
        while let Some(cmd) = cmd_rx.recv().await {
            if process_cmd(cmd, &mut sess, &evt_tx).await {
                break;
            }
        }
        sess
    });
    (cmd_tx, evt_rx, handle)
}

/// Poll until the mock has observed `n` `chat_stream` calls (an in-flight or
/// settled turn's observable footprint).
async fn wait_for_calls(mock: &MockChatClient, n: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while mock.call_count() < n {
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for {n} mock calls, got {}",
                mock.call_count()
            );
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Drain buffered events, then poll until `pred` matches the accumulated
/// batch (or panic on timeout). Returns everything seen so far.
async fn wait_for_events<F>(
    rx: &mut mpsc::Receiver<UiEvent>,
    mut pred: F,
    what: &str,
) -> Vec<UiEvent>
where
    F: FnMut(&[UiEvent]) -> bool,
{
    let mut seen: Vec<UiEvent> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        while let Ok(ev) = rx.try_recv() {
            seen.push(ev);
        }
        if pred(&seen) {
            return seen;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for {what}; saw {} events", seen.len());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// plan→act BackTab while the plan turn is still in flight is intercepted
/// with zero side effects; re-pressed at the idle boundary it hands off.
#[tokio::test]
async fn plan_backtab_blocked_while_running_then_handoff_after_idle() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "plan-gate".into(),
            agent: Some("plan".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    // Call 1: the in-flight plan turn, parked on a hanging LLM stream.
    // Call 2: the post-handoff act turn.
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_hang(hang.clone())
            .push_script(vec![text_done("starting implementation now")]),
    );
    let dir = tempfile::tempdir().unwrap();
    let sess = SessionState::new(
        "plan-gate",
        resolve_agent("plan").unwrap(),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock.clone(),
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());
    // Seed a planning transcript + phase state (mirrors plan_act_handoff.rs).
    let mut sess = sess;
    sess.messages = vec![
        Message::user("u1", "implement feature X"),
        assistant_with_text("a1", "let me explore the codebase first..."),
        assistant_with_text("a2", "## Plan\n1. do X\n2. do Y"),
    ];
    sess.plan_input_count = 1;
    sess.plan_snapshot = Some("## Plan\n1. do X\n2. do Y".into());

    let (cmd_tx, mut evt_rx, worker) = spawn_worker(sess).await;

    // "Enter": the requirement turn starts and parks on the hanging call.
    cmd_tx
        .send(UiCmd::Prompt("implement feature X".into(), Vec::new()))
        .await
        .unwrap();
    wait_for_calls(&mock, 1).await;

    // "BackTab" while running: the direction-aware app-loop gate intercepts
    // plan→act — NOTHING is sent on the command channel. The interception
    // window must leave no observable footprint. (LlmRoundStart is emitted
    // right before the LLM call, so seeing it proves the turn is mid-run.)
    let blocked = wait_for_events(
        &mut evt_rx,
        |evs| {
            evs.iter()
                .any(|e| matches!(e, UiEvent::Session(SessionEvent::LlmRoundStart { .. })))
        },
        "LlmRoundStart of the parked plan turn",
    )
    .await;
    assert_eq!(
        mock.call_count(),
        1,
        "no act turn may start while the plan turn is in flight"
    );
    assert!(
        !blocked
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::AgentSwitch(_)))),
        "interception window must emit no AgentSwitch"
    );
    assert!(
        !blocked
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::TranscriptReset(_)))),
        "interception window must emit no TranscriptReset"
    );
    assert!(
        !blocked
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::PlanHandoff(_)))),
        "interception window must emit no PlanHandoff"
    );

    // Idle boundary: release the hanging stream; the turn settles (stream
    // ends without completion → Error) and TurnDone(plan) arrives.
    hang.notify_one();
    let mut events = wait_for_events(
        &mut evt_rx,
        |evs| {
            evs.iter()
                .any(|e| matches!(e, UiEvent::TurnDone(a) if a == "plan"))
        },
        "TurnDone(plan)",
    )
    .await;
    assert_eq!(mock.call_count(), 1, "still exactly one LLM call at idle");

    // "BackTab" re-pressed at the idle boundary: the handoff fires.
    cmd_tx
        .send(UiCmd::SwitchAndStart("act".into(), "".into()))
        .await
        .unwrap();
    cmd_tx.send(UiCmd::Quit).await.unwrap();
    let sess = worker.await.unwrap();

    let settled = wait_for_events(
        &mut evt_rx,
        |evs| {
            evs.iter()
                .any(|e| matches!(e, UiEvent::TurnDone(a) if a == "act"))
        },
        "TurnDone(act) after handoff",
    )
    .await;
    events.extend(settled);

    // The session is now act — in memory AND persisted.
    assert_eq!(sess.agent.name, "act");
    let meta = store
        .get_session("plan-gate")
        .await
        .unwrap()
        .expect("session row exists");
    assert_eq!(meta.agent.as_deref(), Some("act"));

    // Handoff events: AgentSwitch(act), TranscriptReset folding to the single
    // plan message, and the PlanHandoff card.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::AgentSwitch(ref n)) if n == "act")),
        "AgentSwitch(act) must follow the idle re-press"
    );
    let reset_body = events
        .iter()
        .find_map(|e| match e {
            UiEvent::Session(SessionEvent::TranscriptReset(msgs)) => {
                assert_eq!(msgs.len(), 1, "reset transcript must hold one message");
                Some(msgs[0].text())
            }
            _ => None,
        })
        .expect("TranscriptReset must be emitted after the idle re-press");
    assert!(
        reset_body.contains("## Plan\n1. do X\n2. do Y"),
        "reset message must carry the final plan, got: {reset_body}"
    );
    assert!(
        !reset_body.contains("explore the codebase first"),
        "planning chatter must be dropped, got: {reset_body}"
    );
    let handoff_plan = events
        .iter()
        .find_map(|e| match e {
            UiEvent::Session(SessionEvent::PlanHandoff(p)) => Some(p.clone()),
            _ => None,
        })
        .expect("PlanHandoff must be emitted");
    assert!(
        handoff_plan.contains("## Plan\n1. do X\n2. do Y"),
        "PlanHandoff must carry the plan text, got: {handoff_plan}"
    );

    // Exactly two LLM calls total: the parked plan turn + the act turn; the
    // act request carries ONLY the handoff message (no planning chatter).
    assert_eq!(mock.call_count(), 2, "handoff starts exactly one act turn");
    let req = &mock.requests()[1];
    let user_content: Vec<&str> = req
        .messages
        .iter()
        .filter(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
        .filter_map(|m| m.get("content").and_then(|v| v.as_str()))
        .collect();
    assert!(
        user_content
            .iter()
            .any(|c| c.contains("## Plan\n1. do X\n2. do Y")),
        "act request must include the plan, got: {user_content:?}"
    );
    assert!(
        !user_content
            .iter()
            .any(|c| c.contains("explore the codebase first")),
        "act request must NOT include planning chatter, got: {user_content:?}"
    );
}

/// act→plan queued while an act turn is in flight: a pure state switch the
/// FIFO worker consumes strictly at the turn boundary — the in-flight turn
/// finishes under act and no second turn is started.
#[tokio::test]
async fn act_to_plan_pure_switch_consumed_at_turn_boundary() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "act-gate".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![text_done("working on it")]));
    let dir = tempfile::tempdir().unwrap();
    let sess = SessionState::new(
        "act-gate",
        resolve_agent("act").unwrap(),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock.clone(),
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());

    let (cmd_tx, mut evt_rx, worker) = spawn_worker(sess).await;

    // "Enter" starts a turn; the act→plan BackTab may be pressed while that
    // turn is still in flight (pure switch, no turn start).
    cmd_tx
        .send(UiCmd::Prompt("do the work".into(), Vec::new()))
        .await
        .unwrap();
    cmd_tx
        .send(UiCmd::SwitchAgent("plan".into()))
        .await
        .unwrap();
    cmd_tx.send(UiCmd::Quit).await.unwrap();
    let sess = worker.await.unwrap();

    let events = wait_for_events(
        &mut evt_rx,
        |evs| {
            evs.iter()
                .any(|e| matches!(e, UiEvent::Session(SessionEvent::AgentSwitch(_))))
                && evs.iter().any(|e| matches!(e, UiEvent::TurnDone(_)))
        },
        "TurnDone + AgentSwitch",
    )
    .await;

    // The in-flight turn finished under act BEFORE the switch landed.
    let turn_done = events
        .iter()
        .position(|e| matches!(e, UiEvent::TurnDone(a) if a == "act"))
        .expect("TurnDone(act) must be emitted");
    let agent_switch = events
        .iter()
        .position(
            |e| matches!(e, UiEvent::Session(SessionEvent::AgentSwitch(ref n)) if n == "plan"),
        )
        .expect("AgentSwitch(plan) must be emitted");
    assert!(
        turn_done < agent_switch,
        "pure switch must be consumed at the turn boundary, not mid-turn"
    );
    assert_eq!(mock.call_count(), 1, "pure switch must not start a turn");

    // Mode flipped in memory and persisted (plan phase reset by the worker).
    assert_eq!(sess.agent.name, "plan");
    assert_eq!(
        sess.plan_input_count, 0,
        "plan phase must be reset on switch"
    );
    let meta = store
        .get_session("act-gate")
        .await
        .unwrap()
        .expect("session row exists");
    assert_eq!(meta.agent.as_deref(), Some("plan"));
}
