//! Running-gate tests for the agent-switch slash dispatch (`/act`,
//! `/plan`, `/clear_context`). A switch while a turn is in flight
//! (`running`) or a subagent is live is intercepted with an explicit busy
//! marker — applying a switch mid-`run_session` would land at an arbitrary
//! partial boundary. Same contract as `/compact`'s `SkipRunning`.
//! Replaces the deleted Shift+Tab machinery tests (pure-switch flow was
//! removed; all switching is prompt-driven now).

use super::*;

use super::super::app_loop_actions::{dispatch_mode_switch, ModeSwitch};

/// Shared harness driving `dispatch_mode_switch` directly.
async fn drive_mode_switch(
    mode: ModeSwitch,
    running: bool,
    subagents_running: u32,
) -> (ChatView, bool, u64, Option<(String, u32)>, mpsc::Receiver<UiCmd>) {
    let mut chat = ChatView {
        subagents_running,
        ..Default::default()
    };
    let mut running = running;
    let mut follow = false;
    let mut sys_tokens = 42u64; // sentinel — Run path must overwrite it
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let workdir = Path::new(".");

    let flow = dispatch_mode_switch(
        mode,
        &cmd_tx,
        &mut cancel,
        &mut running,
        &mut follow,
        &mut chat,
        &mut sys_tokens,
        &mut mode_flash,
        0,
        workdir,
    )
    .await;
    assert!(matches!(flow, LoopFlow::Proceed));
    (chat, running, sys_tokens, mode_flash, cmd_rx)
}

/// A turn in flight blocks the switch commands: no command sent, `running`
/// unchanged, and a `[switch] busy` marker is pushed. (ClearContext is not
/// here — it arms the countdown guard instead of hitting the gate; firing
/// while running queues, see `app_loop_dispatch_cmd_tests/act_clear.rs`.)
#[tokio::test]
async fn mode_switch_while_running_is_busy_gated() {
    for mode in [ModeSwitch::Act, ModeSwitch::Plan] {
        let (chat, running, sys_tokens, mode_flash, mut cmd_rx) =
            drive_mode_switch(mode, true, 0).await;
        assert!(running, "running must stay true (turn still active)");
        assert_eq!(sys_tokens, 42, "sys_tokens must be untouched on the block");
        assert!(mode_flash.is_none(), "no flash on the block");
        assert!(
            cmd_rx.try_recv().is_err(),
            "no command should be sent while running"
        );
        assert!(
            chat.blocks
                .iter()
                .any(|b| matches!(b, crate::chat::ChatBlock::Marker(lines)
                if lines.iter().any(|l| l.to_string().contains("busy")))),
            "a [switch] busy marker must be pushed for {mode:?}"
        );
    }
}

/// A live subagent blocks the switch even when `running` is false.
#[tokio::test]
async fn mode_switch_while_subagent_live_is_busy_gated() {
    for mode in [ModeSwitch::Act, ModeSwitch::Plan] {
        let (chat, running, _, _, mut cmd_rx) = drive_mode_switch(mode, false, 1).await;
        assert!(!running);
        assert!(
            cmd_rx.try_recv().is_err(),
            "no command should be sent while a subagent is live"
        );
        assert!(
            chat.blocks
                .iter()
                .any(|b| matches!(b, crate::chat::ChatBlock::Marker(lines)
                if lines.iter().any(|l| l.to_string().contains("busy")))),
            "a [switch] busy marker must be pushed for {mode:?}"
        );
    }
}

/// From idle, each switch command submits its control-command prompt after
/// the ResetCancel preamble, sets the sys-token baseline and the mode flash,
/// and flips the local running/follow state.
#[tokio::test]
async fn mode_switch_from_idle_submits_control_prompt() {
    for (mode, prompt) in [(ModeSwitch::Act, "/act"), (ModeSwitch::Plan, "/plan")] {
        let (chat, running, sys_tokens, mode_flash, mut cmd_rx) =
            drive_mode_switch(mode, false, 0).await;
        assert!(running, "the switch turn starts immediately");
        assert!(
            sys_tokens != 42,
            "sys_tokens baseline must be recomputed for {prompt}"
        );
        let flash = mode_flash.expect("mode flash shows the switch");
        assert!(
            flash.0.contains(prompt.trim_start_matches('/')),
            "flash should name the target mode; got {:?}",
            flash.0
        );
        let first = cmd_rx.try_recv().unwrap();
        assert!(matches!(first, UiCmd::ResetCancel(_)));
        match cmd_rx.try_recv().unwrap() {
            UiCmd::Prompt(text, _) => assert_eq!(text, prompt),
            other => panic!("expected Prompt({prompt}), got {other:?}"),
        }
        assert!(chat.blocks.is_empty(), "no busy marker on the Run path");
    }
}
