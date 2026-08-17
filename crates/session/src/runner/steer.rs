use opencoder_store::Delivery;
use tokio_util::sync::CancellationToken;

use crate::SessionState;

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
pub(super) async fn claim_steers(session: &mut SessionState) -> Vec<(i64, String, Vec<String>)> {
    let Some(store) = session.store.clone() else {
        return Vec::new();
    };
    let sid = session.id.clone();
    // Snapshot the hard cancel token so we can race the DB op without holding a
    // borrow on `session` across the `select!`.
    let hard = session.cancel.clone();
    tokio::select! {
        biased;
        _ = cancel_guard(hard) => Vec::new(),
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
}
