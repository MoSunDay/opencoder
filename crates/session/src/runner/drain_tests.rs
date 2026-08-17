//! Queue/drain-consumption tests extracted from `steer.rs` to keep that
//! file within the repository's per-file size gate.

use super::super::test_fixtures::{make_session, session_with_pending, session_with_queue};
use super::{
    claim_one_queued, drain_mode_step, drain_one_queued, has_pending_queues, idle_drain,
    DrainModeAction, DrainOutcome, IdleAction,
};
use opencoder_core::Role;
use opencoder_store::Delivery;
use tokio_util::sync::CancellationToken;

// ---- Bug 1: drain_mode_step must Proceed on trailing Role::Tool ----

#[tokio::test]
async fn drain_mode_step_proceeds_when_transcript_ends_with_tool_result() {
    // Bug 1: after a tool call executes in drain_mode, the transcript ends
    // with Role::Tool. drain_mode_step must return Proceed (not Idle) so
    // the model processes the tool result instead of stranding it.
    let (mut session, _store) = make_session("drain-tool-test").await;

    session.messages.push(opencoder_core::Message {
        id: "tool-1".into(),
        role: Role::Tool,
        blocks: vec![],
        model: None,
        agent: None,
        usage: opencoder_core::MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    });

    let action = drain_mode_step(&mut session, &mut |_| {}, None)
        .await
        .unwrap();
    assert!(matches!(action, DrainModeAction::Proceed));
}

#[tokio::test]
async fn drain_mode_step_idles_when_transcript_ends_with_assistant() {
    let (mut session, _store) = make_session("drain-asst-test").await;

    session
        .messages
        .push(opencoder_core::Message::assistant("asst-1"));

    let action = drain_mode_step(&mut session, &mut |_| {}, None)
        .await
        .unwrap();
    assert!(matches!(action, DrainModeAction::Idle));
}

#[tokio::test]
async fn has_pending_queues_returns_true_when_turn_cancel_fired() {
    let (session, _store, token) = session_with_pending().await;
    // Fire turn_cancel — the peek must still detect the pending queue.
    token.lock().unwrap().cancel();

    let result = has_pending_queues(&session).await;
    assert!(
        result,
        "has_pending_queues must detect pending queues even when turn_cancel is fired"
    );
}

#[tokio::test]
async fn claim_one_queued_claims_even_when_turn_cancel_fired() {
    let (mut session, _store, token) = session_with_pending().await;
    // Pre-fire turn_cancel: claim_one_queued must still pop the queue.
    token.lock().unwrap().cancel();

    let result = claim_one_queued(&mut session).await;
    let (seq, prompt, _imgs) =
        result.expect("claim_one_queued must pop the pending queue even when turn_cancel is fired");
    assert_eq!(prompt, "queued");
    assert!(seq > 0);
}

// ---- drain_one_queued: single-pop semantics ----
#[tokio::test]
async fn drain_one_queued_bare_control_cmd_returns_control_cmd() {
    let (mut session, _store, _token) = session_with_queue(&["/plan"]).await;
    let mut events = Vec::new();
    let outcome = drain_one_queued(&mut session, &mut |e| events.push(e))
        .await
        .unwrap();
    assert!(
        matches!(outcome, DrainOutcome::ControlCmd),
        "bare /plan should return ControlCmd, got {outcome:?}"
    );
    // Queue should still have zero items after one pop.
    let outcome2 = drain_one_queued(&mut session, &mut |e| events.push(e))
        .await
        .unwrap();
    assert!(
        matches!(outcome2, DrainOutcome::Empty),
        "empty queue should return Empty, got {outcome2:?}"
    );
}

#[tokio::test]
async fn drain_one_queued_real_prompt_returns_prompt() {
    let (mut session, _store, _token) = session_with_queue(&["hello world"]).await;
    let outcome = drain_one_queued(&mut session, &mut |_| {}).await.unwrap();
    assert!(
        matches!(outcome, DrainOutcome::Prompt),
        "real prompt should return Prompt, got {outcome:?}"
    );
}

#[tokio::test]
async fn drain_one_queued_compound_returns_prompt() {
    let (mut session, _store, _token) = session_with_queue(&["/plan review"]).await;
    let outcome = drain_one_queued(&mut session, &mut |_| {}).await.unwrap();
    assert!(
        matches!(outcome, DrainOutcome::Prompt),
        "compound /plan review should return Prompt, got {outcome:?}"
    );
}

#[tokio::test]
async fn drain_one_queued_empty_queue_returns_empty() {
    let (mut session, _store, _token) = session_with_queue(&[]).await;
    let outcome = drain_one_queued(&mut session, &mut |_| {}).await.unwrap();
    assert!(
        matches!(outcome, DrainOutcome::Empty),
        "empty queue should return Empty, got {outcome:?}"
    );
}

#[tokio::test]
async fn drain_one_queued_compound_clear_context_returns_prompt() {
    // `/act_clear_context review` queued: the clear is applied and "review"
    // is recorded as a real prompt so the LLM runs it in the fresh context.
    let (mut session, _store, _token) = session_with_queue(&["/act_clear_context review"]).await;
    let outcome = drain_one_queued(&mut session, &mut |_| {}).await.unwrap();
    assert!(
        matches!(outcome, DrainOutcome::Prompt),
        "compound clear_context should return Prompt, got {outcome:?}"
    );
    // "review" was recorded as a user message (not the raw command).
    let has_review = session
        .messages
        .iter()
        .any(|m| m.role == Role::User && m.text().contains("review"));
    assert!(has_review, "'review' recorded as a user prompt");
    // Agent switched to act.
    assert_eq!(session.agent.name, "act");
}

// ---- idle_drain + hard-cancel regression (Fixes 1 & 2) ----

#[tokio::test]
async fn idle_drain_consumes_pending_queue() {
    let (mut session, store, _) = session_with_queue(&["late-msg"]).await;
    let action = idle_drain(&mut session, &mut |_| {}, None).await.unwrap();
    assert!(matches!(action, IdleAction::Continue));
    assert!(store
        .pending_inputs(&session.id, Delivery::Queue)
        .await
        .unwrap()
        .is_empty());
    assert!(session
        .messages
        .iter()
        .any(|m| m.role == Role::User && m.text().contains("late-msg")));
}

#[tokio::test]
async fn idle_drain_empty_queue_no_gate_returns_done() {
    let (mut session, _, _) = session_with_queue(&[]).await;
    assert!(matches!(
        idle_drain(&mut session, &mut |_| {}, None).await.unwrap(),
        IdleAction::Done
    ));
}

#[tokio::test]
async fn claim_one_queued_completes_under_hard_cancel() {
    let (mut session, store, _) = session_with_queue(&["survives-cancel"]).await;
    let hard = CancellationToken::new();
    hard.cancel();
    session.cancel = Some(hard);
    let (seq, prompt, _) = claim_one_queued(&mut session).await.unwrap();
    assert_eq!(prompt, "survives-cancel");
    assert!(seq > 0);
    assert!(store
        .pending_inputs(&session.id, Delivery::Queue)
        .await
        .unwrap()
        .is_empty());
}
