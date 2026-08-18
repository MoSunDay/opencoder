//! F2: durable input-delivery bookkeeping for the runner.
//!
//! The store tracks every admitted input through a promote → consume →
//! record lifecycle: promotion (claim) makes a row invisible to pending
//! polls, and only `mark_inputs_recorded` confirms it was durably consumed.
//! These helpers wire the runner to that lifecycle — per-item marking at
//! consume time, plus entry recovery that flips promoted-but-unrecorded rows
//! back to pending. Both are best-effort: a store failure is logged, never
//! fatal, because an unmarked row is always recoverable.

use crate::SessionState;

/// Recover orphaned inputs at run entry. Rows promoted-but-never-recorded by
/// a crashed or hard-cancelled prior run are flipped back to pending so this
/// run re-absorbs them; safe because only one drain runs per session at a
/// time (single-writer invariant).
pub(super) async fn recover_orphaned_inputs(session: &SessionState) {
    let Some(store) = session.store.clone() else {
        return;
    };
    let sid = session.id.clone();
    match store.recover_orphan_inputs(&sid).await {
        Ok(n) if n > 0 => {
            tracing::info!(recovered = n, session = %sid, "recovered orphaned inputs");
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, session = %sid, "recover_orphan_inputs failed"),
    }
}

/// Mark one input durably consumed. Marking each input immediately after its
/// consumption closes the promote→consume gap so a crash cannot orphan it; an
/// unmarked row is recoverable, never lost.
pub(super) async fn mark_input_recorded(session: &SessionState, seq: i64) {
    let Some(store) = session.store.clone() else {
        return;
    };
    let sid = session.id.clone();
    if let Err(e) = store.mark_inputs_recorded(&sid, &[seq]).await {
        tracing::warn!(error = %e, session = %sid, seq, "mark_inputs_recorded failed");
    }
}

/// Reset a failed batch's rows (the failed item plus everything not yet
/// processed) back to pending so the next run re-absorbs them. Best-effort:
/// rows left promoted-but-unrecorded by a failed write here are flipped back
/// by [`recover_orphaned_inputs`] at the next run's entry.
pub(super) async fn unpromote_batch(session: &SessionState, seqs: &[i64]) {
    let Some(store) = session.store.clone() else {
        return;
    };
    let sid = session.id.clone();
    let seqs = seqs.to_vec();
    if let Err(e) = store.unpromote_inputs(&sid, &seqs).await {
        tracing::warn!(error = %e, session = %sid, "unpromote_inputs failed");
    }
}
