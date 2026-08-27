use anyhow::Result;
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
/// contended db_lock cannot block the session (hard) cancel indefinitely: if
/// the token fires while a DB read is blocked on the store's serializing
/// Mutex, the read is abandoned (returns its default) and the run loop picks
/// up the cancel at its next boundary.
pub(super) async fn cancel_guard(token: Option<CancellationToken>) {
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
///
/// Split cancel semantics, mirroring `claim_one_queued`: the read-only
/// `pending_inputs` poll IS cancel-guarded (a contended db_lock must not block
/// the hard cancel, and abandoning a read is harmless -- the rows stay
/// pending). The mutating `promote_inputs` is NOT raced with cancel:
/// `promote_inputs` runs BEGIN -> UPDATE promoted_seq -> COMMIT in a manual
/// transaction, and a biased select dropping that future mid-transaction could
/// leave the UPDATE already committed while we return empty -- the rows would
/// be permanently promoted (invisible to future `promoted_seq IS NULL`
/// queries) yet never claimed: permanent data loss. So promote always runs to
/// completion once a non-empty pending read returns; the run loop picks up the
/// cancel at its next boundary.
pub(super) async fn claim_steers(session: &mut SessionState) -> Vec<(i64, String, Vec<String>)> {
    let Some(store) = session.store.clone() else {
        return Vec::new();
    };
    let sid = session.id.clone();
    // Snapshot the hard cancel token so we can race the read without holding a
    // borrow on `session` across the `select!`.
    let hard = session.cancel.clone();
    // Read-only poll, cancel-guarded: on hard cancel we abandon only the read
    // -- nothing has been mutated, so the rows remain pending and recoverable.
    let pending = tokio::select! {
        biased;
        _ = cancel_guard(hard) => Vec::new(),
        v = store.pending_inputs(&sid, Delivery::Steer) => match v {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "claim_steers: pending_inputs failed");
                Vec::new()
            }
        },
    };
    if pending.is_empty() {
        return Vec::new();
    }
    // No cancel-guard here. promote_inputs runs BEGIN -> UPDATE promoted_seq
    // -> COMMIT; racing the hard cancel via a biased select could drop the
    // future mid-COMMIT, leaving the rows permanently promoted (invisible to
    // future queries) yet never claimed -- permanent data loss. Abandoning the
    // read above can only ever leave rows PENDING, never promoted-but-unclaimed.
    let max_seq = pending.iter().map(|i| i.admitted_seq).max().unwrap_or(0);
    let promoted_seqs = match store.promote_inputs(&sid, max_seq, Delivery::Steer).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "claim_steers: promote_inputs failed");
            return Vec::new();
        }
    };
    match_promoted(&pending, &promoted_seqs)
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

/// Outcome of applying a claimed steer batch inside [`run_loop`]
/// (super::mod).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SteerApplyOutcome {
    /// Batch fully applied; run_loop proceeds to the LLM/drain step.
    /// `recorded` tells the caller whether any item became a real user
    /// message (used by the skip-LLM idle-drain decision).
    Continue { recorded: bool },
    /// Sentinel ClearContext or bare-control-command-only batch: `Done` was
    /// emitted, run_loop must end the turn without an LLM call.
    Done,
    /// Hard cancel hit mid-batch: the remaining items (current included)
    /// were unpromoted for the next explicit run and `Status("interrupted")`
    /// was emitted; run_loop must stop.
    Cancelled,
}

/// Apply a claimed steer batch: each item is recorded as a real user turn
/// (or a control command applied inline, never leaking to the LLM), and the
/// outcome tells [`run_loop`] whether to go idle or continue.
///
/// Hard-cancel guard, checked at the top of EVERY item BEFORE the
/// `SteerConsumed` event: the TUI mirror removes a steer row on
/// `SteerConsumed` while a cancelled run suppresses the `Done` resync, so
/// emitting the event and then unpromoting would leave the badge gone but
/// the store row pending — a UI inconsistency. The guard unpromotes the
/// current item plus everything not yet processed (P1-3 batch semantics)
/// and stops the run; the next explicit submission re-absorbs them. A
/// pre-fired cancel cannot reach here (the guarded `claim_steers` read
/// abandons it), so the guard only fires mid-batch — the already-consumed
/// items stay applied, matching the external-async-cancel vs sub-ms-apply
/// boundary. Turn-level (`turn_cancel`) interrupts are intentionally NOT
/// guarded: "submit now" is a queued-into-steer promotion, not a hard stop.
pub(super) async fn apply_steer_batch(
    session: &mut SessionState,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
    steer_prompts: &[(i64, String, Vec<String>)],
) -> Result<SteerApplyOutcome> {
    // Track whether the last steer was a sentinel ClearContext so we
    // can go idle without an LM call.
    let mut clear_sentinel = false;
    let mut steer_recorded = false;
    for (idx, (seq, p, imgs)) in steer_prompts.iter().enumerate() {
        if session.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            // Hard cancel: leave the current item and all remaining items
            // pending so the next explicit run re-absorbs them (mirrors the
            // P1-3 apply-failure recovery below).
            let remaining: Vec<i64> = steer_prompts[idx..].iter().map(|(s, _, _)| *s).collect();
            super::input_recovery::unpromote_batch(session, &remaining).await;
            on_event(SessionEvent::Status("interrupted".into()));
            // Terminal frame owed here too: the caller breaks on Cancelled
            // and nothing else emits `Done` (real-browser acceptance).
            on_event(SessionEvent::Done);
            return Ok(SteerApplyOutcome::Cancelled);
        }
        on_event(SessionEvent::SteerConsumed {
            seq: *seq,
            text: p.clone(),
        });
        // Defensive: a steered control command is applied immediately and
        // NOT recorded as user text, so "/plan" never leaks to the LLM.
        if let Some((cmd, rest)) = crate::control_cmd::split_control_prefix(p) {
            if let Err(e) = crate::control_cmd::apply(session, &cmd, &mut *on_event).await {
                // P1-3: unpromote the failed item and all remaining
                // unprocessed items so the next run re-absorbs them.
                let remaining: Vec<i64> = steer_prompts[idx..].iter().map(|(s, _, _)| *s).collect();
                super::input_recovery::unpromote_batch(session, &remaining).await;
                return Err(e);
            }
            clear_sentinel = matches!(cmd, crate::control_cmd::ControlCmd::ClearContext)
                && crate::control_cmd::is_clear_context_handoff(
                    session.handoff_plan.as_deref().unwrap_or(""),
                );
            // Compound (/plan review): record the rest as a real
            // user message in the new mode.
            if let Some(rest) = rest {
                clear_sentinel = false;
                crate::skill_resolve::record_compound(session, &rest, imgs).await;
                steer_recorded = true;
            }
            // F2: mark per-item (not per-batch): a mid-batch failure
            // leaves earlier items marked, failed+remaining unpromoted.
            super::input_recovery::mark_input_recorded(session, *seq).await;
            continue;
        }
        clear_sentinel = false;
        // Resolve `$skill` tokens, apply plan tag, record as real user turn.
        crate::skill_resolve::record_compound(session, p, imgs).await;
        super::input_recovery::mark_input_recorded(session, *seq).await;
        steer_recorded = true;
    }
    // Sentinel ClearContext: go idle without an LM call.
    if clear_sentinel {
        on_event(SessionEvent::Done);
        return Ok(SteerApplyOutcome::Done);
    }
    // Bare control command(s) only (e.g. a bare "/plan" steer with no
    // accompanying text): the mode/skill switch is the whole intent
    // and no new user message was recorded. Avoid a wasteful LLM call
    // on the existing transcript — go idle, mirroring the initial-
    // prompt short-circuit in `run_with_registry`. Only fires when
    // steers were actually claimed this turn (an empty steer batch
    // with a pending `skip` is handled by the idle-drain block below).
    if !steer_prompts.is_empty() && !steer_recorded {
        on_event(SessionEvent::Done);
        return Ok(SteerApplyOutcome::Done);
    }
    Ok(SteerApplyOutcome::Continue {
        recorded: steer_recorded,
    })
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
    use super::super::test_fixtures::session_with_pending;
    use super::{claim_steers, has_pending_steers, match_promoted};
    use opencoder_store::{Delivery, SessionInput};
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
        let pending = vec![input(1, "alpha"), input(2, "beta"), input(3, "gamma")];
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
        let pending = vec![input(1, "alpha"), input(2, "beta"), input(3, "gamma")];
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

    #[tokio::test]
    async fn claim_steers_claims_even_when_turn_cancel_fired() {
        let (mut session, _store, token) = session_with_pending().await;
        // Pre-fire turn_cancel: claim_* must NOT observe turn_cancel (only the
        // hard cancel). A fired turn_cancel with no active turn is either stale
        // or signals new input was just admitted -- both must claim normally.
        token.lock().unwrap().cancel();

        let steers = claim_steers(&mut session).await;
        assert_eq!(
            steers.len(),
            1,
            "claim_steers must promote pending steer even when turn_cancel is fired"
        );
        assert_eq!(steers[0].1, "interrupt!");
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
    async fn claim_steers_ignores_turn_cancel_and_is_idempotent() {
        let (mut session, _store, token) = session_with_pending().await;

        // Fire turn_cancel then claim: must still promote (turn_cancel is
        // invisible to claim_*).
        token.lock().unwrap().cancel();
        let steers = claim_steers(&mut session).await;
        assert_eq!(steers.len(), 1);

        // Replace the token with a fresh (un-fired) one -- claim must now find
        // nothing left, because the steer was already promoted (idempotent).
        *token.lock().unwrap() = CancellationToken::new();
        let steers = claim_steers(&mut session).await;
        assert!(
            steers.is_empty(),
            "already-promoted steer must not be re-claimed"
        );
    }

    #[tokio::test]
    async fn pre_fired_hard_cancel_leaves_steer_pending_not_lost() {
        let (mut session, store, _token) = session_with_pending().await;
        // Attach the hard cancel token (the fixture only wires turn_cancel),
        // then pre-fire it BEFORE claim_steers runs. With a biased select the
        // pre-fired cancel wins the race against the pending_inputs read: the
        // read is abandoned and claim_steers returns empty. The invariant this
        // pins down: abandoning can only ever leave rows PENDING -- promote
        // never runs, so the row is still visible to future
        // `promoted_seq IS NULL` queries and recoverable. It must NEVER end up
        // promoted-but-unclaimed (silently lost), which the old shape risked by
        // racing promote_inputs against the cancel inside the same select.
        session = session.with_cancel(CancellationToken::new());
        session.cancel.as_ref().unwrap().cancel();

        let steers = claim_steers(&mut session).await;
        assert!(
            steers.is_empty(),
            "pre-fired hard cancel must abandon the read and claim nothing"
        );

        let still_pending = store
            .pending_inputs(&session.id, Delivery::Steer)
            .await
            .unwrap();
        assert_eq!(
            still_pending.len(),
            1,
            "steer row must remain pending (recoverable), not lost"
        );
        assert!(
            still_pending[0].promoted_seq.is_none(),
            "row must keep promoted_seq NULL so a later claim_steers can still pick it up"
        );
        assert_eq!(still_pending[0].prompt, "interrupt!");
    }
}
