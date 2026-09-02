//! Submit-always / apply-at-idle tests for the agent-switch slash dispatch
//! (`/act`, `/plan`). The switch can always be submitted, but it only TAKES
//! EFFECT at a non-running boundary: idle starts the control-command turn
//! now; while a turn is in flight (`running`) the raw command text queues
//! verbatim and the runner applies it via the idle-boundary drain intercept
//! (steer/queue semantics — same contract as `fire_clear_confirm`'s running
//! arm). A live subagent does not count as busy: the parent session is idle,
//! exactly when steer/queue entries are consumed automatically. Ctrl+T and
//! Shift+Tab (act→plan) feed this same dispatch after key handling; slash
//! commands and the shortcuts therefore share persistence, gating, and
//! status updates.

use super::*;

use super::super::app_loop_actions::{dispatch_mode_switch, ModeSwitch};

#[test]
fn mode_switch_target_maps_primary_agent_names() {
    assert_eq!(ModeSwitch::for_agent("act"), ModeSwitch::Act);
    assert_eq!(ModeSwitch::for_agent("plan"), ModeSwitch::Plan);
}

/// Shared harness driving `dispatch_mode_switch` directly. Returns the queue
/// mirror and the admission channel so the running-path tests can assert the
/// queued-row (submit-now, apply-at-idle) contract.
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
    Vec<(i64, String)>,
    mpsc::Receiver<crate::queue_admitter::AdmitReq>,
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
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let (admit_tx, admit_rx) = mpsc::channel(8);
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;

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
        "test",
        &admit_tx,
        &mut admit_st,
        &mut queue_items,
        &mut pending_images,
        &mut history,
        &mut hist_idx,
    )
    .await;
    assert!(matches!(flow, LoopFlow::Proceed));
    (chat, running, sys_tokens, mode_flash, cmd_rx, queue_items, admit_rx)
}

/// A turn in flight queues the switch instead of applying it (steer/queue
/// semantics: submit always, take effect at the idle boundary): the raw
/// command text lands in the queue mirror + admit channel, no `UiCmd` is
/// sent, `running`/`sys_tokens`/flash stay untouched — the switch has not
/// landed yet. (ClearContext is not here — it arms the countdown guard;
/// firing while running queues, see `app_loop_dispatch_cmd_tests/act_clear.rs`.)
#[tokio::test]
async fn mode_switch_while_running_queues_for_idle_boundary() {
    for (mode, prompt) in [(ModeSwitch::Act, "/act"), (ModeSwitch::Plan, "/plan")] {
        let (chat, running, sys_tokens, mode_flash, mut cmd_rx, queue_items, mut admit_rx) =
            drive_mode_switch(mode, true, 0).await;
        assert!(running, "running must stay true (turn still active)");
        assert_eq!(sys_tokens, 42, "sys_tokens untouched: switch not applied yet");
        assert!(mode_flash.is_none(), "no mode flash: switch not landed yet");
        assert!(
            cmd_rx.try_recv().is_err(),
            "no command should be sent while running"
        );
        assert_eq!(
            queue_items,
            vec![(-1, prompt.to_string())],
            "the raw {prompt} row must be queued for the idle boundary"
        );
        let req = admit_rx.try_recv().expect("the admit request must fire");
        assert_eq!(req.display, prompt);
        assert!(
            !chat.blocks.iter().any(|b| matches!(b, crate::chat::ChatBlock::Marker(lines)
            if lines.iter().any(|l| l.to_string().contains("busy")))),
            "no busy refusal marker: the submit always lands (queued) for {mode:?}"
        );
    }
}

/// A live subagent does not gate the switch: the parent session is idle
/// (`running == false`), exactly when steer/queue entries are consumed
/// automatically — so the switch applies now via the Run arm.
#[tokio::test]
async fn mode_switch_with_live_subagent_runs_at_parent_idle_boundary() {
    for (mode, prompt) in [(ModeSwitch::Act, "/act"), (ModeSwitch::Plan, "/plan")] {
        let (chat, running, _, _, mut cmd_rx, _, _) = drive_mode_switch(mode, false, 1).await;
        assert!(running, "the switch turn starts at the idle parent boundary");
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
        let (chat, running, sys_tokens, mode_flash, mut cmd_rx, _, mut admit_rx) =
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
        assert!(
            admit_rx.try_recv().is_err(),
            "the idle Run path must not queue"
        );
    }
}
