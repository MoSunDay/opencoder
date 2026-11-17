//! Off-loop `TranscriptReset` rebuild actor. A mid-run reset rebuild
//! (`session_ui::rebuild_after_reset`) reads the session meta, subagent
//! records and messages and prefetches images — worst case several seconds.
//! Running that inline in the UI select loop froze input, frames and the
//! clear-confirm countdown. This module moves ONE rebuild at a time onto a
//! `tokio::spawn` task; the UI keeps ticking and installs the rebuilt view
//! when it lands.
//!
//! Ordering contract: `fold_ui_events` stops draining the event channel at a
//! local `TranscriptReset`, so every event after the reset stays queued (in
//! order, bounded) until the rebuilt view is installed — the fold then
//! resumes against the fresh view. Remote resets still replay synchronously
//! (no spawn).
//!
//! Cancellation safety: `wait_done` only clears its pending slot AFTER the
//! oneshot resolves (or is dropped), so a cancelled select arm re-polls into
//! the same receiver instead of losing the rebuild. A panicking rebuild task
//! closes the oneshot, which surfaces as `Err` — the slot is cleared and the
//! loop unblocks instead of dead-locking on a guard that never reopens.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use opencoder_core::Message;
use opencoder_store::Store;

use crate::chat::ChatView;

/// One in-flight rebuild: the session it belongs to (staleness guard) and
/// the channel the spawned task delivers the rebuilt view through.
struct Pending {
    session_id: String,
    rx: tokio::sync::oneshot::Receiver<ChatView>,
}

/// UI-side handle for at most one in-flight reset rebuild. `inflight` is a
/// separate `Arc<AtomicBool>` so the app select loop can guard the event arm
/// without borrowing this state alongside the done arm.
pub(crate) struct ResetRebuildState {
    pending: Option<Pending>,
    inflight: Arc<AtomicBool>,
}

impl ResetRebuildState {
    /// Fresh state plus the shared guard flag the app loop selects on.
    pub(crate) fn new() -> (Self, Arc<AtomicBool>) {
        let inflight = Arc::new(AtomicBool::new(false));
        (
            Self {
                pending: None,
                inflight: Arc::clone(&inflight),
            },
            inflight,
        )
    }

    /// Move one `rebuild_after_reset` onto a spawned task. Clones the view,
    /// the reset message list and the store handle; the rebuilt view comes
    /// back through [`Self::wait_done`]. Replaces (drops) any earlier
    /// in-flight rebuild — the UI event arm is guarded while one runs, so a
    /// second spawn can only come from a stale, uninstalled generation.
    pub(crate) fn spawn(
        &mut self,
        chat: &ChatView,
        msgs: &[Message],
        store: &Arc<dyn Store>,
        session_id: &str,
    ) {
        self.pending = None;
        let (tx, rx) = tokio::sync::oneshot::channel::<ChatView>();
        let mut rebuilt = chat.clone();
        let msgs = msgs.to_vec();
        let store = Arc::clone(store);
        let sid = session_id.to_string();
        let task_sid = sid.clone();
        tokio::spawn(async move {
            crate::session_ui::rebuild_after_reset(&mut rebuilt, &msgs, &store, &task_sid).await;
            // A dropped receiver (invalidated/superseded rebuild) is fine:
            // nobody installs a stale view, the task simply finishes.
            let _ = tx.send(rebuilt);
        });
        self.pending = Some(Pending {
            session_id: sid,
            rx,
        });
        self.inflight.store(true, Ordering::Relaxed);
    }

    /// Test seam: simulate a rebuild task that dies before delivering —
    /// the sender is dropped without sending, the exact closed-channel
    /// signal a panicking task produces.
    #[cfg(test)]
    pub(crate) fn spawn_dying(&mut self, session_id: &str) {
        self.pending = None;
        let (tx, rx) = tokio::sync::oneshot::channel::<ChatView>();
        let sid = session_id.to_string();
        tokio::spawn(async move {
            drop(tx); // die before delivering
        });
        self.pending = Some(Pending {
            session_id: sid,
            rx,
        });
        self.inflight.store(true, Ordering::Relaxed);
    }

    /// Resolve the in-flight rebuild. `None` when nothing is pending, when
    /// the rebuild belongs to another session (dropped — e.g. a `/task`
    /// switch raced the rebuild), or when the task died before delivering
    /// (oneshot closed). On success clears the slot, reopens the guard and
    /// returns the rebuilt view for the caller to install.
    pub(crate) async fn wait_done(&mut self, current_session: &str) -> Option<ChatView> {
        if self.pending.as_ref().map(|p| p.session_id.as_str()) != Some(current_session) {
            if self.pending.is_some() {
                tracing::warn!(
                    session = current_session,
                    "dropping stale TranscriptReset rebuild for another session"
                );
            }
            self.invalidate();
            return None;
        }
        // Borrow the receiver in place: a cancelled select arm must find the
        // slot still occupied on its next poll.
        let rx = &mut self.pending.as_mut().expect("pending checked above").rx;
        let delivered = rx.await;
        self.invalidate();
        match delivered {
            Ok(view) => Some(view),
            Err(_) => {
                tracing::warn!("TranscriptReset rebuild task died before delivering the view");
                None
            }
        }
    }

    /// Drop any in-flight rebuild and reopen the event arm. Used on session
    /// switches: a rebuild for the previous session must never install.
    pub(crate) fn invalidate(&mut self) {
        self.pending = None;
        self.inflight.store(false, Ordering::Relaxed);
    }

    /// Whether a rebuild is waiting to be installed (test-facing probe).
    #[cfg(test)]
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
}

impl Default for ResetRebuildState {
    fn default() -> Self {
        Self {
            pending: None,
            inflight: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rebuild task dying before delivery must surface as `None` (warn +
    /// slot cleared + guard reopened) — never a deadlock on a guard that
    /// stays set forever.
    #[tokio::test]
    async fn dying_rebuild_task_unblocks_instead_of_deadlocking() {
        let (mut st, inflight) = ResetRebuildState::new();
        st.spawn_dying("sess-dying");
        assert!(st.is_pending());
        assert!(
            inflight.load(Ordering::Relaxed),
            "event arm is guarded while the rebuild is in flight"
        );
        assert!(
            st.wait_done("sess-dying").await.is_none(),
            "a closed oneshot surfaces as None (warn + clear)"
        );
        assert!(!st.is_pending(), "the pending slot is cleared");
        assert!(
            !inflight.load(Ordering::Relaxed),
            "the event arm reopens after the task died"
        );
    }
}
