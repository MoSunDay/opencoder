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
    let store = match session.store.clone() {
        Some(s) => s,
        None => return Vec::new(),
    };
    let pending = match store.pending_inputs(&session.id, Delivery::Steer).await {
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
    // `promote_inputs` returns the promoted rows' PK seqs (`SELECT seq ...
    // ORDER BY admitted_seq ASC`) -- the same ordering `pending_inputs` uses,
    // so the two vectors align 1:1. Pair each PK with its prompt rather than
    // using `admitted_seq`, so `SteerConsumed` carries the identity the TUI
    // stored via `admit_input`'s return value.
    let promoted_seqs = match store
        .promote_inputs(&session.id, max_seq, Delivery::Steer)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "claim_steers: promote_inputs failed");
            return Vec::new();
        }
    };
    // Match promoted seqs to their input data by identity (PK seq) instead of
    // blindly zipping. A concurrent delete/insert between the two independent DB
    // calls (pending_inputs and promote_inputs each acquire db_lock separately)
    // can make the vectors differ in length; zip would silently misalign prompts
    // with the wrong seq, corrupting history. Seq-match is robust: it pairs each
    // promoted PK with its authoritative data, skipping any seq whose data row
    // vanished. The promoted_seqs vector is authoritative (those rows were
    // actually marked promoted).
    if promoted_seqs.len() != pending.len() {
        tracing::warn!(
            pending_count = pending.len(),
            promoted_count = promoted_seqs.len(),
            "claim_steers: pending/promote length mismatch (concurrent input change?), using seq-match"
        );
    }
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

/// Claim exactly one queued input at idle. Returns its (row seq, prompt), or None.
pub(super) async fn claim_one_queued(
    session: &mut SessionState,
) -> Option<(i64, String, Vec<String>)> {
    let store = session.store.clone()?;
    match store.claim_next_queue(&session.id).await {
        Ok(Some((seq, input))) => Some((seq, input.prompt, input.images.clone())),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, "claim_one_queued failed");
            None
        }
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
    match store.pending_inputs(&session.id, Delivery::Steer).await {
        Ok(v) => !v.is_empty(),
        Err(e) => {
            tracing::warn!(error = %e, "has_pending_steers: pending_inputs failed");
            false
        }
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
    use super::match_promoted;
    use opencoder_store::{Delivery, SessionInput};

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
}
