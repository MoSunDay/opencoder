//! Regression guard for the "Esc then can't submit" bug.
//!
//! The bug: after a double-Esc abort the session's `cancel` token stays
//! permanently cancelled, so `run_loop`'s top-of-loop `is_cancelled()` check
//! breaks immediately on every subsequent submission — the new turn never runs.
//! The TUI loop recovers by swapping in a fresh token (`UiCmd::ResetCancel`)
//! before each turn. This test pins the session-layer invariant that recovery
//! depends on: with a cancelled token a turn no-ops, but after reattaching a
//! fresh, uncancelled token the SAME session runs a real turn end-to-end.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use tokio_util::sync::CancellationToken;

fn done_event(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls: Vec::<CompletedToolCall>::new(),
        usage: Some(Usage {
            input_tokens: 5,
            output_tokens: 3,
            total_tokens: 8,
        }),
    }
}

fn mock_done(text: &str) -> Arc<dyn ChatStream> {
    Arc::new(MockChatClient::new().with_default(vec![done_event(text)]))
}

fn new_session(cancel: CancellationToken) -> SessionState {
    let agent = resolve_agent("act").expect("act agent");
    SessionState::new(
        "cancel-reset",
        agent,
        Config {
            model: "main/glm-5.2".into(),
            ..Config::default()
        },
        mock_done("after-reset-reply"),
        std::env::temp_dir(),
    )
    .with_cancel(cancel)
}

#[tokio::test]
async fn cancelled_token_skips_turn_then_reset_lets_it_run() {
    // --- Phase 1: a cancelled token makes run_loop break at the top, emitting
    //     only Status("interrupted") and recording no assistant message. This
    //     reproduces the post-Esc state where submission appears to do nothing. ---
    let stale = CancellationToken::new();
    stale.cancel();
    let mut s = new_session(stale);
    let mut events: Vec<SessionEvent> = Vec::new();
    run(&mut s, "first".into(), |ev| events.push(ev))
        .await
        .unwrap();

    let saw_interrupted = events
        .iter()
        .any(|ev| matches!(ev, SessionEvent::Status(msg) if msg == "interrupted"));
    assert!(
        saw_interrupted,
        "cancelled token must short-circuit the turn"
    );
    assert!(
        s.messages
            .iter()
            .all(|m| m.role == opencoder_core::Role::User),
        "no assistant turn should be recorded while cancelled"
    );

    // --- Phase 2: swap in a fresh, uncancelled token (exactly what
    //     UiCmd::ResetCancel does in the TUI worker) and run again. The turn
    //     must now execute and append the assistant reply. ---
    let fresh = CancellationToken::new();
    assert!(
        !fresh.is_cancelled(),
        "precondition: fresh token uncancelled"
    );
    s.cancel = Some(fresh);

    events.clear();
    run(&mut s, "second".into(), |ev| events.push(ev))
        .await
        .unwrap();

    let assistant_text: String = s
        .messages
        .iter()
        .rev()
        .find(|m| m.role == opencoder_core::Role::Assistant)
        .map(|m| m.text())
        .unwrap_or_default();
    assert_eq!(
        assistant_text, "after-reset-reply",
        "after resetting the token the turn must run and record the reply"
    );
    assert!(
        events.iter().any(|ev| matches!(ev, SessionEvent::Done)),
        "the reset turn must complete with Done"
    );
}
