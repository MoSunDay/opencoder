use anyhow::Result;
use opencoder_core::Role;
use opencoder_store::Delivery;
use tokio_util::sync::CancellationToken;

use crate::{SessionEvent, SessionState};

/// Resolves when the session is cancelled. If no token is attached, this never
/// resolves (pending forever), so the `select!` cancel arm stays dormant.
pub(crate) async fn await_cancel(session: &SessionState) {
    match session.cancel.as_ref() {
        Some(c) => c.cancelled().await,
        None => std::future::pending::<()>().await,
    }
}

/// A future that resolves when the given token fires, or never resolves when
/// the token is `None`. Used as a cancel-guard arm in `select!` so a
/// contended db_lock cannot block cancel/turn_cancel indefinitely: if the
/// token fires while a DB read is blocked on the store's serializing Mutex,
/// the read is abandoned (returns its default) and the run loop picks up the
/// cancel at its next boundary.
async fn cancel_guard(token: Option<CancellationToken>) {
    match token {
        Some(t) => t.cancelled().await,
        None => std::future::pending().await,
    }
}

/// Claim all pending steer inputs at a turn boundary: read them, mark promoted,
/// return their `(row seq, prompt)` pairs to be appended to history. The row
/// seq is the `session_inputs` primary key -- the same identity `admit_input`
/// returns and the TUI stores in its `steer_items` mirror -- so a
/// `SteerConsumed` event lets the TUI drop the row by identity instead of
/// leaving a stale `steer` row until `Done`. This is NOT the per-session
/// `admitted_seq` (a different column scoped per session). No-op when no store
/// is attached or none pending. Idempotent (promote only touches NULL
/// promoted_seq).
pub(super) async fn claim_steers(session: &mut SessionState) -> Vec<(i64, String, Vec<String>)> {
    let Some(store) = session.store.clone() else {
        return Vec::new();
    };
    let sid = session.id.clone();
    // Snapshot cancel tokens so we can race the DB op without holding a
    // borrow on `session` across the `select!`.
    let hard = session.cancel.clone();
    let turn = session
        .turn_cancel
        .as_ref()
        .and_then(|t| t.lock().ok().map(|g| g.clone()));
    tokio::select! {
        biased;
        _ = cancel_guard(hard) => Vec::new(),
        _ = cancel_guard(turn) => Vec::new(),
        v = async {
            let pending = match store.pending_inputs(&sid, Delivery::Steer).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "claim_steers: pending_inputs failed");
                    return Vec::new();
                }
            };
            if pending.is_empty() {
                return Vec::new();
            }
            let max_seq = pending.iter().map(|i| i.admitted_seq).max().unwrap_or(0);
            let promoted_seqs = match store
                .promote_inputs(&sid, max_seq, Delivery::Steer)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "claim_steers: promote_inputs failed");
                    return Vec::new();
                }
            };
            match_promoted(&pending, &promoted_seqs)
        } => v,
    }
}

/// Match promoted PK seqs to their authoritative input data by identity.
/// Robust against concurrent insertion/deletion between the two independent
/// `pending_inputs` and `promote_inputs` DB calls: instead of blindly zipping
/// (which misaligns when lengths differ), each promoted seq is paired with its
/// matching input row, skipping any seq whose data vanished.
fn match_promoted(
    pending: &[opencoder_store::SessionInput],
    promoted_seqs: &[i64],
) -> Vec<(i64, String, Vec<String>)> {
    promoted_seqs
        .iter()
        .filter_map(|&seq| {
            pending
                .iter()
                .find(|i| i.seq == Some(seq))
                .map(|i| (seq, i.prompt.clone(), i.images.clone()))
        })
        .collect()
}

/// Claim exactly one queued input at idle. Returns its (row seq, prompt), or None.
pub(super) async fn claim_one_queued(
    session: &mut SessionState,
) -> Option<(i64, String, Vec<String>)> {
    let store = session.store.clone()?;
    let sid = session.id.clone();
    let hard = session.cancel.clone();
    let turn = session
        .turn_cancel
        .as_ref()
        .and_then(|t| t.lock().ok().map(|g| g.clone()));
    tokio::select! {
        biased;
        _ = cancel_guard(hard) => None,
        _ = cancel_guard(turn) => None,
        v = async {
            match store.claim_next_queue(&sid).await {
                Ok(Some((seq, input))) => Some((seq, input.prompt, input.images.clone())),
                Ok(None) => None,
                Err(e) => {
                    tracing::warn!(error = %e, "claim_one_queued failed");
                    None
                }
            }
        } => v,
    }
}

/// Peek (read-only) whether any Steer inputs are pending for this session,
/// WITHOUT promoting them. Used at the idle boundary (text-only turn, empty
/// queue) to close the race where a steer is admitted after the top-of-loop
/// `claim_steers` but before `Done` would otherwise strand it. Returns false
/// when no store is attached or the read fails (fail-open: go idle).
pub(super) async fn has_pending_steers(session: &SessionState) -> bool {
    let Some(store) = session.store.clone() else {
        return false;
    };
    let sid = session.id.clone();
    let hard = session.cancel.clone();
    tokio::select! {
        biased;
        _ = cancel_guard(hard) => false,
        v = async {
            match store.pending_inputs(&sid, Delivery::Steer).await {
                Ok(v) => !v.is_empty(),
                Err(e) => {
                    tracing::warn!(error = %e, "has_pending_steers: pending_inputs failed");
                    false
                }
            }
        } => v,
    }
}

/// Outcome of popping exactly one queued input at a turn/idle boundary.
#[derive(Debug)]
pub(super) enum DrainOutcome {
    /// A real prompt (or compound command rest, or ClearContext sentinel)
    /// was consumed and recorded. The caller should proceed to an LLM turn.
    Prompt,
    /// A bare control command was applied inline (agent switch etc.) with
    /// no real prompt. The caller should skip the LLM call and drain the
    /// next item on the following loop iteration.
    ControlCmd,
    /// The queue is empty — nothing was popped.
    Empty,
}

/// Pop exactly **one** queued input at an idle/turn boundary. Applies bare
/// control commands inline (no LLM turn), records real prompts via
/// [`crate::skill_resolve::record_compound`].
///
/// Unlike the previous `drain_queued` which looped internally and could pop
/// multiple items per call (bare commands via `continue`), this pops at most
/// one item. The caller re-invokes on the next loop iteration (skipping the
/// LLM call) to drain subsequent items, giving the outer loop a chance to
/// check for interrupts and new steers between each pop.
pub(super) async fn drain_one_queued(
    session: &mut SessionState,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<DrainOutcome> {
    if let Some((seq, q, imgs)) = claim_one_queued(session).await {
        on_event(SessionEvent::QueueConsumed { seq, text: q.clone() });
        if let Some((cmd, rest)) = crate::control_cmd::split_control_prefix(&q) {
            crate::control_cmd::apply(session, &cmd, &mut *on_event).await?;
            // ClearContext with a preserved result breaks to execute it;
            // sentinel path (no result) forces an outer iteration.
            if matches!(cmd, crate::control_cmd::ControlCmd::ClearContext)
                && !crate::control_cmd::is_clear_context_handoff(
                    session.handoff_plan.as_deref().unwrap_or(""),
                )
            {
                return Ok(DrainOutcome::Prompt);
            }
            // Compound (/plan review): rest is a real prompt in the new
            // mode — record it and break.
            if let Some(rest) = rest {
                crate::skill_resolve::record_compound(session, &rest, &imgs).await;
                return Ok(DrainOutcome::Prompt);
            }
            // Bare command: applied, no LLM turn needed.
            return Ok(DrainOutcome::ControlCmd);
        }
        // Real prompt: resolve `$skill` tokens, record, break.
        crate::skill_resolve::record_compound(session, &q, &imgs).await;
        return Ok(DrainOutcome::Prompt);
    }
    // Queue empty.
    Ok(DrainOutcome::Empty)
}

/// Action the caller should take after an idle-boundary drain.
pub(super) enum IdleAction {
    /// A prompt (or late steer/queue) was found — continue the outer loop.
    Continue,
    /// A bare control command was applied — skip the next LLM call and
    /// drain again.
    SkipLlm,
    /// Queue empty and no late steer/queue — emit Done and stop.
    Done,
}

/// Drain one queued item at an idle boundary and determine the next action.
/// Encapsulates pop-one + late-steer/queue peek so [`run_loop`] can call it
/// from both the normal idle path and the skip-LLM path without duplication.
pub(super) async fn idle_drain(
    session: &mut SessionState,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<IdleAction> {
    match drain_one_queued(session, on_event).await? {
        DrainOutcome::Prompt => Ok(IdleAction::Continue),
        DrainOutcome::ControlCmd => Ok(IdleAction::SkipLlm),
        DrainOutcome::Empty => {
            let late_steer = has_pending_steers(session).await;
            let late_queue = !late_steer && has_pending_queues(session).await;
            if late_steer || late_queue {
                Ok(IdleAction::Continue)
            } else {
                Ok(IdleAction::Done)
            }
        }
    }
}

/// Action for `run_loop` after a drain-mode pre-consume step.
pub(super) enum DrainModeAction {
    /// A real prompt was consumed (or transcript needs a response) — proceed
    /// to the LLM call.
    Proceed,
    /// A bare command was applied, or a late steer/queue appeared — loop back.
    ConsumeNext,
    /// Queue empty, nothing pending — go idle.
    Idle,
}

/// One step of drain-mode pre-consume: pop a queued input and decide whether
/// to proceed to the LLM call, loop back for the next item, or go idle.
/// Called only when `drain_mode` is active and no steers are pending.
pub(super) async fn drain_mode_step(
    session: &mut SessionState,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<DrainModeAction> {
    match drain_one_queued(session, on_event).await? {
        DrainOutcome::Prompt => Ok(DrainModeAction::Proceed),
        DrainOutcome::ControlCmd => Ok(DrainModeAction::ConsumeNext),
        DrainOutcome::Empty => {
            // Queue empty. If the transcript ends with an unresponded user
            // message (e.g., a plan→act handoff awaiting execution), proceed
            // to the LLM call. Exclude the clear-context fresh-start sentinel.
            let needs_llm = session
                .messages
                .last()
                .is_some_and(|m| m.role == Role::User)
                && !session
                    .handoff_plan
                    .as_deref()
                    .is_some_and(crate::control_cmd::is_clear_context_handoff);
            if needs_llm {
                return Ok(DrainModeAction::Proceed);
            }
            // Late-check: a steer/queue may have been admitted after the pop.
            if has_pending_steers(session).await || has_pending_queues(session).await {
                Ok(DrainModeAction::ConsumeNext)
            } else {
                Ok(DrainModeAction::Idle)
            }
        }
    }
}

/// Peek (read-only) whether any Queue inputs are pending for this session,
/// WITHOUT claiming them. Used at the idle boundary (text-only turn, empty
/// queue) to close the race where a queued input is admitted after
/// `claim_one_queued` returns None but before `Done` would strand it.
/// Symmetric with [`has_pending_steers`]. Returns false when no store is
/// attached or the read fails (fail-open: go idle).
pub(super) async fn has_pending_queues(session: &SessionState) -> bool {
    let Some(store) = session.store.clone() else {
        return false;
    };
    let sid = session.id.clone();
    let hard = session.cancel.clone();
    tokio::select! {
        biased;
        _ = cancel_guard(hard) => false,
        v = async {
            match store.pending_inputs(&sid, Delivery::Queue).await {
                Ok(v) => !v.is_empty(),
                Err(e) => {
                    tracing::warn!(error = %e, "has_pending_queues: pending_inputs failed");
                    false
                }
            }
        } => v,
    }
}

/// Resolves when a turn-level interrupt is requested. Like `await_cancel` but
/// for the separate `turn_cancel` token: resolves when a subagent steer
/// "submit-now" (the `>` button) fires the token. Stays pending forever when
/// no token is attached (parent sessions).
pub(crate) async fn await_turn_cancel(session: &SessionState) {
    let token = session
        .turn_cancel
        .as_ref()
        .and_then(|t| t.lock().ok().map(|g| g.clone()));
    match token {
        Some(tc) => tc.cancelled().await,
        None => std::future::pending::<()>().await,
    }
}

/// Check whether the turn-level interrupt token has been fired.
pub(crate) fn is_turn_cancelled(session: &SessionState) -> bool {
    session
        .turn_cancel
        .as_ref()
        .and_then(|t| t.lock().ok())
        .map(|g| g.is_cancelled())
        .unwrap_or(false)
}

/// Reset the turn-level interrupt token (replace with a fresh one) so the next
/// turn starts with a clean, uncancelled token.
pub(crate) fn reset_turn_cancel(session: &mut SessionState) {
    if let Some(tc) = &session.turn_cancel {
        if let Ok(mut g) = tc.lock() {
            *g = CancellationToken::new();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{claim_one_queued, claim_steers, drain_one_queued, has_pending_queues, has_pending_steers, match_promoted, DrainOutcome};
    use crate::SessionState;
    use crate::SharedCancel;
    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
    use opencoder_store::{Delivery, LibsqlStore, SessionInput, Store};
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    fn input(seq: i64, prompt: &str) -> SessionInput {
        SessionInput {
            seq: Some(seq),
            id: format!("id-{seq}"),
            session_id: "s1".into(),
            delivery: Delivery::Steer,
            prompt: prompt.into(),
            images: vec![],
            admitted_seq: seq,
            promoted_seq: None,
            display_text: None,
        }
    }

    #[test]
    fn match_promoted_aligns_by_seq_not_position() {
        // 3 pending inputs; promoted only 2 (simulating concurrent delete of
        // seq 2 between pending_inputs and promote_inputs). Zip would pair
        // seq 3 with "beta" (position 1); seq-match correctly pairs it with
        // "gamma".
        let pending = vec![
            input(1, "alpha"),
            input(2, "beta"),
            input(3, "gamma"),
        ];
        let promoted = vec![1_i64, 3];
        let result = match_promoted(&pending, &promoted);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], (1, "alpha".into(), vec![]));
        assert_eq!(result[1], (3, "gamma".into(), vec![]));
    }

    #[test]
    fn match_promoted_skips_missing_seqs() {
        // promoted seq 2 doesn't exist in pending (row vanished)
        let pending = vec![input(1, "alpha")];
        let promoted = vec![1_i64, 2];
        let result = match_promoted(&pending, &promoted);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], (1, "alpha".into(), vec![]));
    }

    #[test]
    fn match_promoted_preserves_happy_path_order() {
        let pending = vec![
            input(1, "alpha"),
            input(2, "beta"),
            input(3, "gamma"),
        ];
        let promoted = vec![1_i64, 2, 3];
        let result = match_promoted(&pending, &promoted);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].0, 1);
        assert_eq!(result[1].0, 2);
        assert_eq!(result[2].0, 3);
    }

    // ---- cancel-guard: pre-fired turn_cancel returns defaults ----
    //
    // These exercise the tokio::select!{ biased; cancel_guard; db_op } wrapper
    // around DB reads. When turn_cancel is pre-fired, the cancel arm wins the
    // biased select and the function returns its default — even though the DB
    // actually has pending items.
    //
    // NOTE: uses an in-memory LibsqlStore. These are pub(super) fns that can
    // only be tested inside the crate. The DB operations complete in <1ms on
    // in-memory SQLite, so timing is not a concern.

    fn mock_client() -> Arc<dyn ChatStream> {
        Arc::new(
            MockChatClient::new().with_default(vec![LlmEvent::Completed {
                text: "ok".into(),
                tool_calls: vec![],
                usage: None,
            }]),
        )
    }

    /// Build a SessionState wired to an in-memory store that already has one
    /// pending Steer input and one pending Queue input.
    async fn session_with_pending() -> (SessionState, Arc<dyn Store>, SharedCancel) {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());

        // Seed the session row (FK constraint on session_inputs).
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "cancel-guard-test".into(),
                title: Some("test".into()),
                agent: Some("act".into()),
                model: Some("m/g".into()),
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
            })
            .await
            .unwrap();

        // Admit one steer and one queue input.
        let steer_input = SessionInput {
            seq: None,
            id: "steer-1".into(),
            session_id: "cancel-guard-test".into(),
            delivery: Delivery::Steer,
            prompt: "interrupt!".into(),
            images: vec![],
            admitted_seq: 0,
            promoted_seq: None,
            display_text: None,
        };
        store.admit_input(&steer_input).await.unwrap();

        let queue_input = SessionInput {
            seq: None,
            id: "queue-1".into(),
            session_id: "cancel-guard-test".into(),
            delivery: Delivery::Queue,
            prompt: "queued".into(),
            images: vec![],
            admitted_seq: 0,
            promoted_seq: None,
            display_text: None,
        };
        store.admit_input(&queue_input).await.unwrap();

        let agent = resolve_agent("act").unwrap();
        let config = Config {
            model: "m/g".into(),
            ..Default::default()
        };
        let token: SharedCancel = Arc::new(std::sync::Mutex::new(CancellationToken::new()));

        let session = SessionState::new(
            "cancel-guard-test",
            agent,
            config,
            mock_client(),
            std::env::temp_dir(),
        )
        .with_store(store.clone())
        .with_turn_cancel(token.clone());

        (session, store, token)
    }

    #[tokio::test]
    async fn claim_steers_returns_empty_when_turn_cancel_pre_fired() {
        let (mut session, _store, token) = session_with_pending().await;
        // Pre-fire turn_cancel
        token.lock().unwrap().cancel();

        let steers = claim_steers(&mut session).await;
        assert!(
            steers.is_empty(),
            "pre-fired turn_cancel must short-circuit claim_steers"
        );
    }

    #[tokio::test]
    async fn has_pending_steers_returns_true_when_turn_cancel_fired() {
        let (session, _store, token) = session_with_pending().await;
        // Fire turn_cancel — a fired turn_cancel signals new input was
        // submitted, so the peek must still detect it.
        token.lock().unwrap().cancel();

        let result = has_pending_steers(&session).await;
        assert!(
            result,
            "has_pending_steers must detect pending steers even when turn_cancel is fired"
        );
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
    async fn claim_one_queued_returns_none_when_turn_cancel_pre_fired() {
        let (mut session, _store, token) = session_with_pending().await;
        // Pre-fire turn_cancel
        token.lock().unwrap().cancel();

        let result = claim_one_queued(&mut session).await;
        assert!(
            result.is_none(),
            "pre-fired turn_cancel must short-circuit claim_one_queued"
        );
    }


    // ---- drain_one_queued: single-pop semantics ----

    async fn session_with_queue(prompts: &[&str]) -> (SessionState, Arc<dyn Store>, SharedCancel) {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "drain-test".into(),
                title: Some("test".into()),
                agent: Some("act".into()),
                model: Some("m/g".into()),
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
            })
            .await
            .unwrap();
        for (i, p) in prompts.iter().enumerate() {
            store
                .admit_input(&SessionInput {
                    seq: None,
                    id: format!("q-{i}"),
                    session_id: "drain-test".into(),
                    delivery: Delivery::Queue,
                    prompt: (*p).into(),
                    images: vec![],
                    admitted_seq: 0,
                    promoted_seq: None,
                    display_text: None,
                })
                .await
                .unwrap();
        }
        let agent = resolve_agent("act").unwrap();
        let config = Config {
            model: "m/g".into(),
            ..Default::default()
        };
        let token: SharedCancel = Arc::new(std::sync::Mutex::new(CancellationToken::new()));
        let session = SessionState::new(
            "drain-test",
            agent,
            config,
            mock_client(),
            std::env::temp_dir(),
        )
        .with_store(store.clone())
        .with_turn_cancel(token.clone());
        (session, store, token)
    }

    #[tokio::test]
    async fn drain_one_queued_bare_control_cmd_returns_control_cmd() {
        let (mut session, _store, _token) = session_with_queue(&["/plan"]).await;
        let mut events = Vec::new();
        let outcome = drain_one_queued(&mut session, &mut |e| events.push(e)).await.unwrap();
        assert!(
            matches!(outcome, DrainOutcome::ControlCmd),
            "bare /plan should return ControlCmd, got {outcome:?}"
        );
        // Queue should still have zero items after one pop.
        let outcome2 = drain_one_queued(&mut session, &mut |e| events.push(e)).await.unwrap();
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
    async fn claim_steers_returns_data_after_turn_cancel_reset() {
        let (mut session, _store, token) = session_with_pending().await;

        // First: pre-fired → empty
        token.lock().unwrap().cancel();
        let steers = claim_steers(&mut session).await;
        assert!(steers.is_empty());

        // Reset token
        *token.lock().unwrap() = CancellationToken::new();

        // Now claim_steers should find the pending steer
        let steers = claim_steers(&mut session).await;
        assert_eq!(steers.len(), 1);
        assert_eq!(steers[0].1, "interrupt!");
    }
}
