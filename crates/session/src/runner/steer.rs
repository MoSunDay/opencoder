use opencoder_store::Delivery;

use crate::SessionState;

/// Resolves when the session is cancelled. If no token is attached, this never
/// resolves (pending forever), so the `select!` cancel arm stays dormant.
pub(super) async fn await_cancel(session: &SessionState) {
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
    let pending = match store
        .pending_inputs(&session.id, Delivery::Steer)
        .await
    {
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
    pending
        .into_iter()
        .zip(promoted_seqs)
        .map(|(i, seq)| (seq, i.prompt, i.images.clone()))
        .collect()
}

/// Claim exactly one queued input at idle. Returns its (row seq, prompt), or None.
pub(super) async fn claim_one_queued(session: &mut SessionState) -> Option<(i64, String, Vec<String>)> {
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
