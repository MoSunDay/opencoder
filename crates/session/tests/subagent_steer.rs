//! Turn-level interrupt (subagent steer) tests — verifies the `turn_cancel`
//! mechanism that lets a parent cut a single LLM turn short so a pending steer
//! can be absorbed at the next turn boundary, while the loop itself keeps
//! running.
//!
//! Contracts:
//! - `is_turn_cancelled` / `reset_turn_cancel` are `pub(crate)`, so they are
//!   unreachable from this integration test. We exercise the same check / fire /
//!   reset cycle against the public `SharedCancel` token they wrap (Test 1), and
//!   observe the full behavior through the public `run()` API (Tests 2 & 3).
//! - A pre-fired `turn_cancel` produces one empty (interrupted) turn, then the
//!   loop continues: the pending steer is consumed and the session completes
//!   with a `Done` event (Test 2).
//! - Sessions without `turn_cancel` (normal parent sessions) behave exactly as
//!   before — a plain turn completes with `Done` (Test 3).
//!
//! Script-consumption note: `MockChatClient::chat_stream` pops the next script
//! the moment `chat_stream` is called, which happens *before* the biased
//! `select!` in `run_one_llm_call` resolves the (already-fired) `turn_cancel`
//! arm. The interrupted turn therefore still consumes one script, so a turn
//! that runs normally afterwards needs its own script — hence Test 2 pushes two.

use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_session::{run, SessionEvent, SessionState, SharedCancel};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, Store};
use tokio_util::sync::CancellationToken;

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

/// A turn that completes with plain text and no tool calls (ends the loop).
fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// Exercise the check / fire / reset cycle of the turn-cancel token directly.
///
/// `is_turn_cancelled` and `reset_turn_cancel` are `pub(crate)`, so this test
/// drives the public `SharedCancel` (an `Arc<Mutex<CancellationToken>>`) the way
/// those helpers do: a fresh token reports not-cancelled, firing it reports
/// cancelled, and swapping in a fresh token (exactly what `reset_turn_cancel`
/// does) flips it back to not-cancelled.
#[test]
fn turn_cancel_helpers_work() {
    let token: SharedCancel = Arc::new(Mutex::new(CancellationToken::new()));

    // Fresh token → not cancelled (mirrors is_turn_cancelled == false).
    assert!(
        !token.lock().unwrap().is_cancelled(),
        "fresh turn_cancel must report not-cancelled"
    );

    // Fire it — this is what the subagent steer "submit-now" (`>`) button does.
    token.lock().unwrap().cancel();
    assert!(
        token.lock().unwrap().is_cancelled(),
        "fired turn_cancel must report cancelled"
    );

    // reset_turn_cancel replaces the inner token with a fresh one.
    *token.lock().unwrap() = CancellationToken::new();
    assert!(
        !token.lock().unwrap().is_cancelled(),
        "after reset, turn_cancel must report not-cancelled again"
    );
}

/// A session whose `turn_cancel` is already fired when `run` starts still
/// completes: the first LLM turn is cut short (empty turn), the loop resets the
/// token and continues, the pending steer is absorbed, and a second turn runs to
/// completion emitting `Done`.
#[tokio::test]
async fn turn_cancel_allows_loop_to_continue() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    // Two scripts: the first turn is interrupted (its script is still consumed
    // because chat_stream pops at call time, before turn_cancel wins the select),
    // the second turn runs normally.
    let mock = MockChatClient::new()
        .push_script(vec![text_done("first turn, will be interrupted")])
        .push_script(vec![text_done("second turn after steer")]);

    let mut session = SessionState::new(
        "test-turn-cancel",
        resolve_agent("act").unwrap(),
        config(),
        Arc::new(mock) as Arc<dyn ChatStream>,
        std::env::temp_dir(),
    );
    session = session.with_store(store.clone());

    // The foreign-key on session_inputs requires the parent session row to
    // exist first. `run` normally creates it via the first `record`, but we
    // admit the steer before running, so create the row explicitly here.
    let meta = SessionMeta {
        id: "test-turn-cancel".to_string(),
        agent: Some("act".to_string()),
        model: Some(config().model),
        ..SessionMeta::default()
    };
    store.create_session(&meta).await.unwrap();

    // Set up the turn_cancel token and fire it immediately, so the first LLM
    // call's biased select! resolves the turn_cancel arm before any stream event.
    let token: SharedCancel = Arc::new(Mutex::new(CancellationToken::new()));
    token.lock().unwrap().cancel();
    session.turn_cancel = Some(token);

    // Admit a steer before running — it should be absorbed at a turn boundary.
    let input = SessionInput {
        seq: None,
        id: "test-steer-1".to_string(),
        session_id: "test-turn-cancel".to_string(),
        delivery: Delivery::Steer,
        prompt: "change direction".to_string(),
        images: vec![],
        admitted_seq: 0,
        promoted_seq: None,
    };
    store.admit_input(&input).await.unwrap();

    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    run(&mut session, "initial prompt".to_string(), move |ev| {
        events_clone.lock().unwrap().push(ev);
    })
    .await
    .unwrap();

    let evs = events.lock().unwrap();

    // The steer should have been consumed (absorbed into history).
    let steer_consumed = evs
        .iter()
        .any(|e| matches!(e, SessionEvent::SteerConsumed { .. }));
    assert!(steer_consumed, "expected SteerConsumed event");

    // The session should have completed — the loop continued past the interrupt.
    let done = evs.iter().any(|e| matches!(e, SessionEvent::Done));
    assert!(
        done,
        "expected Done event — loop continued after turn interrupt"
    );
}

/// Sessions without `turn_cancel` (normal parent sessions) work exactly as
/// before: a single text-only turn completes and emits `Done`.
#[tokio::test]
async fn turn_cancel_not_set_behaves_normally() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = MockChatClient::new().push_script(vec![text_done("normal response")]);

    let mut session = SessionState::new(
        "test-no-turn-cancel",
        resolve_agent("act").unwrap(),
        config(),
        Arc::new(mock) as Arc<dyn ChatStream>,
        std::env::temp_dir(),
    );
    session = session.with_store(store);
    // turn_cancel stays None (default) — a normal, non-subagent session.

    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    run(&mut session, "hello".to_string(), move |ev| {
        events_clone.lock().unwrap().push(ev);
    })
    .await
    .unwrap();

    let evs = events.lock().unwrap();
    assert!(
        evs.iter().any(|e| matches!(e, SessionEvent::Done)),
        "expected Done event for a turn_cancel-less session"
    );
}
