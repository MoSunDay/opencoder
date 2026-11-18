//! Regression tests for the queue-panel `>` (steer submit-now) click on a
//! REMOTE session. `steer_fire::handle_steer_submit` fires only the LOCAL
//! `turn_cancel` token, which the remote worker never observes mid-turn —
//! so the click was a silent no-op server-side. The fix forwards an
//! `UiCmd::Interrupt` to the worker (translated into a server `interrupt`
//! command) whenever the outcome was `SteerOnly` and no LIVE subagent block
//! holds the focus (a live child's `>` must keep targeting that child).

use super::*;
use crate::chat::ChatBlock;
use crate::chat::ChatView;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type SharedCancel = Arc<Mutex<CancellationToken>>;

fn fresh_turn_cancel() -> SharedCancel {
    Arc::new(Mutex::new(CancellationToken::new()))
}

fn empty_child_runtime() -> crate::worker::ChildRuntimeHandles {
    crate::worker::ChildRuntimeHandles {
        cancels: Arc::new(Mutex::new(HashMap::new())),
        turn_cancels: Arc::new(Mutex::new(HashMap::new())),
        steer_gates: Arc::new(Mutex::new(HashMap::new())),
    }
}

/// Remote + running parent + pending steer: the `>` click must surface as a
/// `UiCmd::Interrupt` on the worker channel. The local turn_cancel firing is
/// harmless on remote sessions and deliberately NOT asserted. No StartTurn
/// fired: `running` stays true and no ResetCancel/Prompt crossed the channel.
#[tokio::test]
async fn remote_running_steer_click_sends_interrupt() {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(4);
    let mut cancel = CancellationToken::new();
    let mut turn_gen = 0u64;
    let mut running = true;
    let mut chat = ChatView {
        remote: true,
        steer_items: vec![(1, "stop now".to_string())],
        ..ChatView::default()
    };
    let mut follow = false;
    let child_runtime = empty_child_runtime();
    let turn_cancel = fresh_turn_cancel();

    steer_submit_after_mouse(
        &cmd_tx,
        &mut cancel,
        &mut turn_gen,
        None,
        &mut running,
        &mut chat,
        &mut follow,
        &child_runtime,
        &turn_cancel,
    )
    .await;

    assert!(
        matches!(cmd_rx.try_recv(), Ok(UiCmd::Interrupt)),
        "remote > click must forward an interrupt to the worker"
    );
    assert!(
        running,
        "no StartTurn fired — running must stay true (nothing else on the channel)"
    );
    assert!(cmd_rx.try_recv().is_err(), "channel must be drained after the Interrupt");
}

/// Local contract unchanged: the same click fires the LOCAL shared turn_cancel
/// (the local worker observes it mid-turn) and sends NO worker command.
#[tokio::test]
async fn local_running_steer_click_sends_no_commands() {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(4);
    let mut cancel = CancellationToken::new();
    let mut turn_gen = 0u64;
    let mut running = true;
    let mut chat = ChatView {
        remote: false,
        steer_items: vec![(1, "stop now".to_string())],
        ..ChatView::default()
    };
    let mut follow = false;
    let child_runtime = empty_child_runtime();
    let turn_cancel = fresh_turn_cancel();

    steer_submit_after_mouse(
        &cmd_tx,
        &mut cancel,
        &mut turn_gen,
        None,
        &mut running,
        &mut chat,
        &mut follow,
        &child_runtime,
        &turn_cancel,
    )
    .await;

    assert!(
        cmd_rx.try_recv().is_err(),
        "local > click must not enqueue any worker command"
    );
    assert!(
        turn_cancel.lock().unwrap().is_cancelled(),
        "local > click is the force-insert contract: turn_cancel must fire"
    );
    assert!(running);
}

/// Live subagent focus: the focused child owns the `>` click, so interrupting
/// the REMOTE PARENT turn would be wrong — no `UiCmd::Interrupt` may be sent.
#[tokio::test]
async fn remote_subagent_focus_click_does_not_send_interrupt() {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(4);
    let mut cancel = CancellationToken::new();
    let mut turn_gen = 0u64;
    let mut running = true;
    let mut chat = ChatView {
        remote: true,
        steer_items: vec![(1, "stop now".to_string())],
        ..ChatView::default()
    };
    chat.blocks.push(ChatBlock::Subagent {
        id: "task-1".into(),
        child_session_id: "sub-1".into(),
        kind: "explore".into(),
        prompt: "p".into(),
        view: ChatView::default(),
        done: false,
        ok: true,
        cancelled: false,
        summary: String::new(),
        started_at_ms: 0,
        elapsed_ms: None,
    });
    let mut follow = false;
    let child_runtime = empty_child_runtime();
    let turn_cancel = fresh_turn_cancel();

    steer_submit_after_mouse(
        &cmd_tx,
        &mut cancel,
        &mut turn_gen,
        Some(0),
        &mut running,
        &mut chat,
        &mut follow,
        &child_runtime,
        &turn_cancel,
    )
    .await;

    assert!(
        cmd_rx.try_recv().is_err(),
        "live subagent focus must not interrupt the remote parent turn"
    );
    assert!(running);
}
