//! Idle-boundary drain re-kick for stranded admits.
//!
//! A queued/steered input can land in the store while the TUI is idle, or in
//! the exact window the runner's turn already passed its final pending check
//! ("admitted but never consumed" — the drain-strand bug). The admitter
//! actor's completion is the last point that knows the row is durable, so the
//! UI re-checks the STORE there and, when idle with pending rows, restarts
//! the drain loop with an empty prompt (drain mode) — mirroring the
//! Done-handler's `drain_pending` re-kick in `app_loop`.

use std::sync::Arc;

use opencoder_store::{Delivery, Store};

/// True when the store still holds a pending queue OR steer row for `sid`.
/// The store is authoritative: UI mirrors can be stale in both directions
/// (optimistic temp rows not yet reconciled; consumed rows not yet folded).
/// A store read error reports no stranded rows (fail-closed) — the next
/// Done/TurnDone boundary re-checks and heals.
pub(crate) async fn stranded_pending(store: &Arc<dyn Store>, sid: &str) -> bool {
    for delivery in [Delivery::Queue, Delivery::Steer] {
        if let Ok(rows) = store.pending_inputs(sid, delivery).await {
            if !rows.is_empty() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_store_reports_no_stranded_rows() {
        let store: Arc<dyn Store> =
            Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "s".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(!stranded_pending(&store, "s").await);
    }

    #[tokio::test]
    async fn pending_queue_row_is_stranded() {
        let store: Arc<dyn Store> =
            Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "s".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        store
            .admit_input(&opencoder_store::SessionInput {
                seq: None,
                id: "i1".into(),
                session_id: "s".into(),
                delivery: Delivery::Queue,
                prompt: "later".into(),
                images: vec![],
                display_text: None,
                admitted_seq: 0,
                promoted_seq: None,
            })
            .await
            .unwrap();
        assert!(stranded_pending(&store, "s").await);
    }

    async fn seeded_store() -> Arc<dyn Store> {
        let store: Arc<dyn Store> =
            Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "s".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        store
    }

    fn ok_done(temp_seq: i64, real_seq: i64) -> crate::queue_admitter::AdmitDone {
        crate::queue_admitter::AdmitDone {
            temp_seq,
            result: Ok(real_seq),
            display: "d".into(),
        }
    }

    /// A1 core: a successful admit landing while IDLE with a stranded pending
    /// row must restart the drain (ResetCancel + empty Prompt) — without it
    /// the row would never be consumed.
    #[tokio::test]
    async fn idle_admit_with_pending_row_restarts_drain() {
        use opencoder_store::Delivery;
        let store = seeded_store().await;
        store
            .admit_input(&opencoder_store::SessionInput {
                seq: None,
                id: "q1".into(),
                session_id: "s".into(),
                delivery: Delivery::Queue,
                prompt: "stranded".into(),
                images: vec![],
                display_text: None,
                admitted_seq: 0,
                promoted_seq: None,
            })
            .await
            .unwrap();
        let mut st = crate::queue_admitter::AdmitUiState::default();
        let mut queue_items = vec![(-1, "stranded".to_string())];
        let mut pending_images = vec![];
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<crate::worker::UiCmd>(8);
        let mut cancel = tokio_util::sync::CancellationToken::new();

        let o = on_admit_done(
            ok_done(-1, 1),
            &mut st,
            &mut queue_items,
            &mut pending_images,
            false, // idle
            &store,
            "s",
            &cmd_tx,
            &mut cancel,
        )
        .await;

        assert_eq!(
            o.flow,
            AdmitDoneFlow::Started,
            "idle stranded admit must restart the drain"
        );
        // Mirror reconciled to the real seq.
        assert_eq!(queue_items, vec![(1, "stranded".to_string())]);
        // Drain restart commands: ResetCancel (fresh token) then empty Prompt.
        assert!(matches!(
            cmd_rx.recv().await.unwrap(),
            crate::worker::UiCmd::ResetCancel(_)
        ));
        match cmd_rx.recv().await.unwrap() {
            crate::worker::UiCmd::Prompt(p, imgs) => {
                assert!(p.is_empty(), "drain re-kick uses an empty prompt, got: {p}");
                assert!(imgs.is_empty());
            }
            _ => panic!("expected empty Prompt, got a different command"),
        }
    }

    /// While a drain is RUNNING the re-kick must not fire (the live drain's
    /// idle boundary consumes the row) — and a FAILED admit is a plain no-op.
    #[tokio::test]
    async fn running_or_failed_admit_does_not_rekick() {
        use opencoder_store::Delivery;
        let store = seeded_store().await;
        store
            .admit_input(&opencoder_store::SessionInput {
                seq: None,
                id: "q1".into(),
                session_id: "s".into(),
                delivery: Delivery::Steer,
                prompt: "steered".into(),
                images: vec![],
                display_text: None,
                admitted_seq: 0,
                promoted_seq: None,
            })
            .await
            .unwrap();
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<crate::worker::UiCmd>(8);
        let mut cancel = tokio_util::sync::CancellationToken::new();

        // Running drain: no re-kick.
        let mut st = crate::queue_admitter::AdmitUiState::default();
        let o = on_admit_done(
            ok_done(-1, 1),
            &mut st,
            &mut vec![],
            &mut vec![],
            true, // running
            &store,
            "s",
            &cmd_tx,
            &mut cancel,
        )
        .await;
        assert_eq!(o.flow, AdmitDoneFlow::Ok);
        assert!(cmd_rx.try_recv().is_err(), "no commands while running");

        // Failed admit (rolled back): no re-kick even when idle.
        let o = on_admit_done(
            crate::queue_admitter::AdmitDone {
                temp_seq: -1,
                result: Err(anyhow::anyhow!("store down")),
                display: "d".into(),
            },
            &mut st,
            &mut vec![],
            &mut vec![],
            false,
            &store,
            "s",
            &cmd_tx,
            &mut cancel,
        )
        .await;
        assert_eq!(o.flow, AdmitDoneFlow::Ok);
        assert!(o.flash.is_some(), "failure surfaces a flash");
        assert!(cmd_rx.try_recv().is_err(), "no commands for a failed admit");
    }

    /// Idle with an EMPTY store (the row was already consumed before the
    /// completion landed): no needless drain turn.
    #[tokio::test]
    async fn idle_admit_with_nothing_pending_is_plain_ok() {
        let store = seeded_store().await;
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<crate::worker::UiCmd>(8);
        let mut cancel = tokio_util::sync::CancellationToken::new();
        let mut st = crate::queue_admitter::AdmitUiState::default();

        let o = on_admit_done(
            ok_done(-1, 1),
            &mut st,
            &mut vec![],
            &mut vec![],
            false,
            &store,
            "s",
            &cmd_tx,
            &mut cancel,
        )
        .await;
        assert_eq!(o.flow, AdmitDoneFlow::Ok);
        assert!(
            cmd_rx.try_recv().is_err(),
            "no drain restart on an empty store"
        );
    }

    #[tokio::test]
    async fn missing_session_reports_no_stranded_rows() {
        let store: Arc<dyn Store> =
            Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
        assert!(!stranded_pending(&store, "no-such-session").await);
    }
}

/// Result of folding an admitter-actor completion at the app-loop select arm.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AdmitDoneFlow {
    /// Nothing further to do (failure rolled back, drain already running,
    /// or no stranded rows remain in the store).
    Ok,
    /// A stranded pending row was found while idle and the drain loop was
    /// restarted — the caller flips `running`/`follow` and begins the turn.
    Started,
    /// The worker command channel is closed: the caller marks the worker
    /// dead and breaks the main loop.
    WorkerDead,
}

/// Combined outcome: the failure flash [`queue_admitter::apply_done`] produced
/// (if any) plus the re-kick decision.
pub(crate) struct AdmitDoneOutcome {
    pub(crate) flash: Option<&'static str>,
    pub(crate) flow: AdmitDoneFlow,
}

/// Fold an `AdmitDone` at the app-loop select arm: reconcile the optimistic
/// queue mirror, then the A1 stranded-admit re-kick — the row is now DURABLE,
/// so if the UI is idle (the runner's turn already ended, possibly right past
/// its final pending check) nothing else will ever consume it. Re-check the
/// STORE and restart the drain loop with an empty prompt (drain mode),
/// mirroring the Done-handler's `drain_pending` re-kick in `app_loop`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn on_admit_done(
    done: crate::queue_admitter::AdmitDone,
    st: &mut crate::queue_admitter::AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    running: bool,
    store: &Arc<dyn Store>,
    session_id: &str,
    cmd_tx: &tokio::sync::mpsc::Sender<crate::worker::UiCmd>,
    cancel: &mut tokio_util::sync::CancellationToken,
) -> AdmitDoneOutcome {
    // Capture the outcome before `apply_done` consumes `done`.
    let admit_ok = done.result.is_ok();
    let flash = crate::queue_admitter::apply_done(st, done, queue_items, pending_images);
    if !admit_ok || running || !stranded_pending(store, session_id).await {
        return AdmitDoneOutcome {
            flash,
            flow: AdmitDoneFlow::Ok,
        };
    }
    let flow = if crate::app_helpers::start_turn(
        cmd_tx,
        cancel,
        crate::worker::UiCmd::Prompt(String::new(), Vec::new()),
    )
    .await
    {
        AdmitDoneFlow::Started
    } else {
        AdmitDoneFlow::WorkerDead
    };
    AdmitDoneOutcome { flash, flow }
}
