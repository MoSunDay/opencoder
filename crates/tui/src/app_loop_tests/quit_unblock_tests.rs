//! Quit-path regression tests for `handle_quit`:
//!
//! - local sessions ALWAYS cancel the shared token — the old code only
//!   cancelled when `busy`, and the `running`/`subagents_running` flags can
//!   read stale-idle while the worker is still inside `run_session`, leaving
//!   `UiCmd::Quit` queued behind the whole turn;
//! - remote sessions keep the detach exemption: a remote quit must NOT
//!   interrupt server-side execution (the server owns the run);
//! - the Quit hand-off is non-blocking (`try_send`): a full channel must
//!   never freeze the UI loop before `quitting` is set.

use super::*;
use crate::chat::ChatView;

/// Stale-idle local quit: `busy == false` but the worker may still be inside
/// `run_session` — the token cancel must fire regardless (NEW behavior).
#[tokio::test]
async fn quit_cancels_local_token_even_when_idle() {
    let cancel = CancellationToken::new();
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(4);
    let mut chat = ChatView {
        remote: false,
        ..ChatView::default()
    };

    handle_quit(false, &cancel, &mut chat, &cmd_tx).await;

    assert!(
        cancel.is_cancelled(),
        "local quit must cancel the shared token even when busy flags read idle"
    );
    assert!(matches!(cmd_rx.try_recv(), Ok(UiCmd::Quit)));
}

/// Existing contract pinned: busy local quit still cancels + delivers Quit.
#[tokio::test]
async fn quit_cancels_local_token_when_busy() {
    let cancel = CancellationToken::new();
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(4);
    let mut chat = ChatView {
        remote: false,
        ..ChatView::default()
    };

    handle_quit(true, &cancel, &mut chat, &cmd_tx).await;

    assert!(cancel.is_cancelled());
    assert!(matches!(cmd_rx.try_recv(), Ok(UiCmd::Quit)));
}

/// Remote detach exemption: quitting a remote session must NOT cancel the
/// token (that would send `interrupt` to the server via the remote worker's
/// `cancel.cancelled()` arm) — the server owns the in-flight run.
#[tokio::test]
async fn quit_does_not_cancel_remote_token() {
    let cancel = CancellationToken::new();
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(4);
    let mut chat = ChatView {
        remote: true,
        ..ChatView::default()
    };

    handle_quit(true, &cancel, &mut chat, &cmd_tx).await;

    assert!(
        !cancel.is_cancelled(),
        "remote detach must not interrupt server-side execution"
    );
    assert!(matches!(cmd_rx.try_recv(), Ok(UiCmd::Quit)));
}

/// Non-blocking hand-off: with a wedged worker and a FULL bounded channel the
/// old blocking `send` froze the UI loop before `quitting` was set. The
/// try_send must return immediately (dropped on `Full`) and the pre-filled
/// command stays the only queued one.
#[tokio::test]
async fn quit_never_blocks_on_a_full_channel() {
    let cancel = CancellationToken::new();
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(1);
    cmd_tx.try_send(UiCmd::Compact).unwrap(); // fill the bounded channel
    let mut chat = ChatView {
        remote: false,
        ..ChatView::default()
    };

    tokio::time::timeout(
        std::time::Duration::from_millis(500),
        handle_quit(true, &cancel, &mut chat, &cmd_tx),
    )
    .await
    .expect("handle_quit must never block on a full command channel");

    // The Quit try_send was dropped on `Full`: the pre-filled Compact is
    // still the only queued command. Termination still completes because
    // `app_bootstrap::finish` drops `cmd_tx` and BOTH worker loops break on
    // channel close (`None`) — the dropped Quit need not be observed.
    assert!(matches!(cmd_rx.try_recv(), Ok(UiCmd::Compact)));
    assert!(cmd_rx.try_recv().is_err());
}
