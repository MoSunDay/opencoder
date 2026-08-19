//! Bidirectional running-gate tests for mode switches (Shift+Tab /
//! SwitchAgentNoClear / `/act` `/plan` `/act_clear_context`). A mode switch
//! in EITHER direction while a turn is in flight (`running`) or a subagent
//! is live is intercepted with an explicit busy hint — applying a switch
//! mid-`run_session` would land at an arbitrary partial boundary (and
//! plan→act would start the next turn with a stale agent), and there is no
//! deferred auto-fire (re-press at a clean idle boundary). Same contract as
//! the slash paths' `worker::gate_switch`.
//! Split out of `app_loop_tests/mod.rs` to keep the aggregator under the
//! 800-line iteration cap (rules/02).

use super::plan_view;
use super::*;

/// Bidirectional running gate, plan→act side: a turn in flight makes the
/// switch intercepted with an explicit busy hint — even plan→act WITHOUT a
/// submitted plan (which would otherwise be a pure switch). The agent stays
/// in plan mode and the user re-presses at a clean idle boundary.
#[tokio::test]
async fn switch_while_running_is_noop_even_without_submitted_plan() {
    let mut chat = ChatView {
        agent: "plan".into(),
        plan_submitted: false,
        ..Default::default()
    };
    let mut running = true;
    let mut follow = true;
    let mut input = "keep me".to_string();
    let mut cursor_idx = 7;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 42u64; // sentinel — must not be touched on the block
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        false,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while running"
    );
    assert!(running, "running must stay true (turn still active)");
    assert_eq!(input, "keep me", "input must be untouched on the block");
    assert_eq!(sys_tokens, 42, "sys_tokens must be untouched on the block");
    assert_eq!(
        chat.agent, "plan",
        "agent must stay in the current mode on the block"
    );
    assert!(
        mode_flash
            .as_ref()
            .map(|(t, _)| t.contains("busy") && t.contains("blocked"))
            .unwrap_or(false),
        "mode flash should state the switch is blocked while busy; got {:?}",
        mode_flash
    );
}

/// Bidirectional gate, act→plan side: a turn in flight makes the switch
/// intercepted with the SAME busy hint as plan→act — no command is sent,
/// running/input/cursor/sys_tokens are untouched, and the agent stays in
/// act mode until the user re-presses at a clean idle boundary.
#[tokio::test]
async fn switch_act_to_plan_while_running_is_noop() {
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut running = true;
    let mut follow = true;
    let mut input = "in flight".to_string();
    let mut cursor_idx = 9;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 7u64; // sentinel — must not be touched on the block
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "plan".into(),
        false,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while running"
    );
    assert!(running, "running must stay true (turn still active)");
    assert_eq!(input, "in flight", "input must be untouched on the block");
    assert_eq!(cursor_idx, 9, "cursor must be untouched on the block");
    assert_eq!(sys_tokens, 7, "sys_tokens must be untouched on the block");
    assert_eq!(
        chat.agent, "act",
        "agent must stay in the current mode on the block"
    );
    assert!(
        mode_flash
            .as_ref()
            .map(|(t, _)| t.contains("busy") && t.contains("blocked"))
            .unwrap_or(false),
        "mode flash should state the switch is blocked while busy; got {:?}",
        mode_flash
    );
}

/// SwitchAgentNoClear (t+Tab chord, no_handoff=true) on the plan→act side
/// while a turn is running is intercepted with the busy hint — even with a
/// submitted plan, where the non-handoff path would otherwise leak a direct
/// UiCmd::SwitchAgent mid-turn. No deferred auto-fire: re-press when idle.
#[tokio::test]
async fn switch_no_clear_while_running_is_noop() {
    let mut chat = plan_view();
    let mut running = true;
    let mut follow = true;
    let mut input = "do not lose me".to_string();
    let mut cursor_idx = 14;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 99u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        true,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while running"
    );
    assert!(running, "running must stay true (turn still active)");
    assert_eq!(
        chat.agent, "plan",
        "agent must stay in plan mode on the intercepted switch"
    );
    assert_eq!(sys_tokens, 99, "sys_tokens must be untouched on the block");
    assert!(
        mode_flash
            .as_ref()
            .map(|(t, _)| t.contains("busy") && t.contains("blocked"))
            .unwrap_or(false),
        "mode flash should state the switch is blocked while busy; got {:?}",
        mode_flash
    );
}

/// SwitchAgentNoClear (no_handoff=true) idle: pure switch, transcript preserved
/// — NO plan→act handoff even with a submitted plan.
#[tokio::test]
async fn switch_no_clear_idle_skips_handoff() {
    let mut chat = plan_view();
    let mut running = false;
    let mut follow = false;
    let mut input = "keep my plan".to_string();
    let mut cursor_idx = 12;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        true,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(!running, "no_handoff must not start a turn");
    assert_eq!(chat.agent, "act", "agent must switch");
    assert_eq!(
        input, "keep my plan",
        "no_handoff must preserve the input (no handoff drain)"
    );
    match cmd_rx.try_recv().unwrap() {
        UiCmd::SwitchAgent(ref n) => assert_eq!(n, "act"),
        _other => panic!("expected SwitchAgent"),
    }
}

/// A subagent task is live (subagents_running > 0) but the loop `running`
/// flag is false - this happens when a SubagentEnd was dropped under channel
/// saturation and Done/TurnDone already cleared `running`. The gate must
/// STILL intercept a plan→act switch (no silent handoff or mode change
/// while a tracked subagent exists); act→plan is intercepted the same way
/// (see `switch_act_to_plan_while_subagent_live_is_noop`).
#[tokio::test]
async fn switch_while_subagent_running_is_noop_even_when_running_false() {
    let mut chat = ChatView {
        agent: "plan".into(),
        subagents_running: 1,
        plan_submitted: false,
        ..Default::default()
    };
    let mut running = false;
    let mut follow = true;
    let mut input = "keep me".to_string();
    let mut cursor_idx = 7;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 42u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        false,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while a subagent is live"
    );
    assert!(!running, "running must stay false (no turn started)");
    assert_eq!(input, "keep me", "input must be untouched on the noop");
    assert_eq!(sys_tokens, 42, "sys_tokens must be untouched on the noop");
    assert_eq!(
        chat.agent, "plan",
        "agent must stay plan while a subagent is live"
    );
    assert!(
        mode_flash
            .as_ref()
            .map(|(t, _)| t.contains("busy"))
            .unwrap_or(false),
        "mode flash should hint the switch is deferred; got {:?}",
        mode_flash
    );
}

// ----- P0 regression: rapid Shift+Tab double-tap (act→plan→act) -----
//
// `plan_submitted` is sticky: it survives the previous plan→act handoff and
// was previously only collapsed when the worker's `AgentSwitch("plan")` event
// round-tripped back to the UI. A second tap inside that window saw
// `plan_to_act && plan_submitted` and fired a bogus `SwitchAndStart` handoff:
// input box drained, act-mode answer fabricated into a "plan", transcript
// collapsed, `handoff_seq` persisted. The optimistic flip now folds the whole
// switch synchronously, so the flag is already false when the second tap runs.

/// First tap (act→plan) must synchronously collapse the sticky flag — even
/// though the worker's `AgentSwitch("plan")` event has NOT arrived yet.
#[tokio::test]
async fn switch_act_to_plan_collapses_stale_plan_submitted_synchronously() {
    let mut chat = ChatView {
        agent: "act".into(),
        // Sticky from the previous plan→act handoff: TranscriptReset keeps it
        // and only a folded AgentSwitch("plan") would clear it.
        plan_submitted: true,
        ..Default::default()
    };
    let mut running = false;
    let mut follow = true;
    let mut input = String::new();
    let mut cursor_idx = 0;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "plan".into(),
        false,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert_eq!(chat.agent, "plan", "agent flipped optimistically");
    assert!(
        !chat.plan_submitted,
        "the optimistic flip must fold the switch and collapse the sticky \
         plan_submitted flag WITHOUT waiting for the AgentSwitch event"
    );
    // Pure-switch routing: exactly one SwitchAgent command, no turn started.
    assert!(matches!(
        cmd_rx.try_recv().unwrap(),
        UiCmd::SwitchAgent(ref n) if n == "plan"
    ));
    assert!(cmd_rx.try_recv().is_err(), "no further command");
    assert!(!running, "act→plan is a pure switch, not a turn");
}

/// Full double-tap regression: tap 2 (plan→act) lands before the worker's
/// `AgentSwitch("plan")` event returns. With the synchronous fold it must
/// route a PURE switch — input preserved, no `SwitchAndStart`, no turn.
#[tokio::test]
async fn shift_tab_double_tap_second_strike_is_pure_switch_and_keeps_input() {
    let mut chat = ChatView {
        agent: "act".into(),
        plan_submitted: true, // sticky from the previous handoff
        ..Default::default()
    };
    let mut running = false;
    let mut follow = true;
    let mut input = "draft note".to_string();
    let mut cursor_idx = 10;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    // Tap 1: act → plan (AgentSwitch event still in flight).
    let outcome = handle_switch_agent(
        "plan".into(),
        false,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;
    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(!chat.plan_submitted, "tap 1 collapsed the sticky flag");

    // Tap 2: plan → act, still inside the event round-trip window.
    let outcome = handle_switch_agent(
        "act".into(),
        false,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert_eq!(
        input, "draft note",
        "pure switch must NOT drain the input box"
    );
    assert_eq!(cursor_idx, 10, "cursor untouched");
    assert!(!running, "no turn started");
    assert_eq!(chat.agent, "act", "agent flipped to act");
    // Worker FIFO receives exactly the two pure switches — never SwitchAndStart.
    let mut cmds = Vec::new();
    while let Ok(c) = cmd_rx.try_recv() {
        cmds.push(c);
    }
    assert_eq!(cmds.len(), 2, "exactly two commands queued");
    assert!(
        cmds.iter().all(|c| matches!(c, UiCmd::SwitchAgent(_))),
        "double-tap must queue pure switches only"
    );
}

/// Bidirectional gate, act→plan with a LIVE subagent (running=false,
/// subagents_running=1): intercepted like every other busy case — no
/// command, agent stays act, sys_tokens/input untouched, busy flash. A
/// tracked subagent alone is enough to block leaving act mode (it mirrors
/// the slash paths' `gate_switch` refusal).
#[tokio::test]
async fn switch_act_to_plan_while_subagent_live_is_noop() {
    let mut chat = ChatView {
        agent: "act".into(),
        subagents_running: 1,
        ..Default::default()
    };
    let mut running = false;
    let mut follow = true;
    let mut input = "keep me".to_string();
    let mut cursor_idx = 7;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 42u64; // sentinel — must not be touched on the block
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "plan".into(),
        false,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while a subagent is live"
    );
    assert!(!running, "running must stay false (no turn started)");
    assert_eq!(input, "keep me", "input must be untouched on the block");
    assert_eq!(sys_tokens, 42, "sys_tokens must be untouched on the block");
    assert_eq!(
        chat.agent, "act",
        "agent must stay act while a subagent is live"
    );
    assert!(
        mode_flash
            .as_ref()
            .map(|(t, _)| t.contains("busy") && t.contains("blocked"))
            .unwrap_or(false),
        "mode flash should state the switch is blocked while busy; got {:?}",
        mode_flash
    );
}

/// Bidirectional gate, act→plan with no_handoff=true (t+Tab chord) while
/// running: intercepted exactly like the plain act→plan path — no command,
/// running/agent/input untouched, busy flash. The chord does not bypass the
/// running gate in either direction.
#[tokio::test]
async fn switch_act_to_plan_no_clear_while_running_is_noop() {
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut running = true;
    let mut follow = true;
    let mut input = "in flight".to_string();
    let mut cursor_idx = 9;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 11u64; // sentinel — must not be touched on the block
    let workdir = Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "plan".into(),
        true,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;

    assert!(matches!(outcome, SwitchOutcome::Proceed));
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while running"
    );
    assert!(running, "running must stay true (turn still active)");
    assert_eq!(
        chat.agent, "act",
        "agent must stay in the current mode on the block"
    );
    assert_eq!(
        input, "in flight",
        "input must be untouched by the intercepted switch"
    );
    assert_eq!(sys_tokens, 11, "sys_tokens must be untouched on the block");
    assert!(
        mode_flash
            .as_ref()
            .map(|(t, _)| t.contains("busy") && t.contains("blocked"))
            .unwrap_or(false),
        "mode flash should state the switch is blocked while busy; got {:?}",
        mode_flash
    );
}
