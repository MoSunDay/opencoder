//! Busy-gate tests for the agent-switch slash dispatch (`/act`, `/plan`).
//! Idle starts the control-command turn now; while a turn is in flight
//! (`running`) the switch is REFUSED with the shared busy flash — a mode
//! switch never applies mid-flight and is never queued. A live subagent does
//! not count as busy: the parent session is idle, exactly when switching is
//! safe. Ctrl+T and Shift+Tab (act→plan) feed this same dispatch after key
//! handling; slash commands and the shortcuts therefore share persistence,
//! gating, and status updates.

use super::*;

use super::super::app_loop_actions::{dispatch_mode_switch, ModeSwitch};

#[test]
fn mode_switch_target_maps_primary_agent_names() {
    assert_eq!(ModeSwitch::for_agent("act"), ModeSwitch::Act);
    assert_eq!(ModeSwitch::for_agent("plan"), ModeSwitch::Plan);
}

/// Shared harness driving `dispatch_mode_switch` directly. Returns the
/// flash so the running-path tests can assert the busy refusal contract.
#[allow(clippy::type_complexity)]
async fn drive_mode_switch(
    mode: ModeSwitch,
    running: bool,
    subagents_running: u32,
) -> (
    ChatView,
    bool,
    u64,
    Option<(String, u32)>,
    mpsc::Receiver<UiCmd>,
) {
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

/// A turn in flight refuses the switch outright: no `UiCmd` is sent,
/// `running`/`sys_tokens` stay untouched, nothing is queued, and the shared
/// busy flash ("任务运行中不可切换状态") names the refusal. (ClearContext is
/// not here — it arms the countdown guard; firing while running queues, see
/// `app_loop_dispatch_cmd_tests/act_clear.rs`.)
#[tokio::test]
async fn mode_switch_while_running_refuses_with_busy_flash() {
    for (mode, prompt) in [(ModeSwitch::Act, "/act"), (ModeSwitch::Plan, "/plan")] {
        let (chat, running, sys_tokens, mode_flash, mut cmd_rx) =
            drive_mode_switch(mode, true, 0).await;
        assert!(running, "running must stay true (turn still active)");
        assert_eq!(
            sys_tokens, 42,
            "sys_tokens untouched: switch not applied"
        );
        let flash = mode_flash.expect("the busy refusal flash must be set");
        assert!(
            flash.0.contains("任务运行中不可切换状态"),
            "flash must name the refusal for {prompt}; got {:?}",
            flash.0
        );
        assert!(
            cmd_rx.try_recv().is_err(),
            "no command should be sent while running"
        );
        assert!(
            chat.blocks.is_empty(),
            "the refusal is a status flash, not a transcript marker, for {mode:?}"
        );
    }
}

/// A live subagent does not gate the switch: the parent session is idle
/// (`running == false`), exactly when steer/queue entries are consumed
/// automatically — so the switch applies now via the Run arm.
#[tokio::test]
async fn mode_switch_with_live_subagent_runs_at_parent_idle_boundary() {
    for (mode, prompt) in [(ModeSwitch::Act, "/act"), (ModeSwitch::Plan, "/plan")] {
        let (chat, running, _, _, mut cmd_rx) = drive_mode_switch(mode, false, 1).await;
        assert!(
            running,
            "the switch turn starts at the idle parent boundary"
        );
        let first = cmd_rx.try_recv().expect("the switch must be submitted");
        assert!(matches!(first, UiCmd::ResetCancel(_)));
        match cmd_rx.try_recv().unwrap() {
            UiCmd::Prompt(text, _) => assert_eq!(text, prompt),
            other => panic!("expected Prompt({prompt}), got {other:?}"),
        }
        assert!(chat.blocks.is_empty(), "no refusal marker on the Run path");
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
        assert!(chat.blocks.is_empty(), "no refusal marker on the Run path");
    }
}
