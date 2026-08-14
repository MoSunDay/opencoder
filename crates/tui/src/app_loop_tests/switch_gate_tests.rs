//! Running-gate tests for mode switches (Shift+Tab / SwitchAgentNoClear /
//! `/act` `/plan` `/act_clear_context`). A turn in flight (`running`) must
//! defer any mode switch to the next clean idle boundary — applying it
//! mid-`run_session` would start the next turn with a stale agent at an
//! arbitrary partial boundary. Split out of `app_loop_tests/mod.rs` to keep
//! the aggregator under the 800-line iteration cap (rules/02).

use super::plan_view;
use super::*;

/// Broadened running-gate: ANY mode switch while a turn is running is a
/// no-op — including act→plan and plan→act WITHOUT a submitted plan (which
/// would otherwise be a pure switch + could start a new turn mid-flight).
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
    let mut sys_tokens = 42u64; // sentinel — must not be touched on the noop
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
    assert_eq!(input, "keep me", "input must be untouched on no-op");
    assert_eq!(sys_tokens, 42, "sys_tokens must be untouched on the noop");
    assert_eq!(
        chat.agent, "plan",
        "agent must stay in the current mode on the noop"
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

/// act→plan while running is likewise a no-op (broadened running-gate covers
/// switches in both directions).
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
    let mut sys_tokens = 7u64;
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
    assert_eq!(chat.agent, "act", "agent must stay in act mode on the noop");
    assert_eq!(sys_tokens, 7, "sys_tokens must be untouched on the noop");
    assert!(
        mode_flash
            .as_ref()
            .map(|(t, _)| t.contains("busy"))
            .unwrap_or(false),
        "mode flash should hint the switch is deferred; got {:?}",
        mode_flash
    );
}

/// SwitchAgentNoClear (t+Tab chord, no_handoff=true) while a turn is running
/// must ALSO be a no-op — even plan→act with a submitted plan, where the
/// non-handoff path would otherwise leak a direct UiCmd::SwitchAgent mid-turn.
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
        "agent must stay in plan mode on the noop"
    );
    assert_eq!(sys_tokens, 99, "sys_tokens must be untouched on the noop");
    assert!(
        mode_flash
            .as_ref()
            .map(|(t, _)| t.contains("busy"))
            .unwrap_or(false),
        "mode flash should hint the switch is deferred; got {:?}",
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
/// STILL block the mode switch (no silent mode change while a tracked
/// subagent exists).
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
