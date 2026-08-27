//! ctrl+t pure mode switch — structurally separated from the Shift+Tab /
//! Alt+Tab handoff path (`app_loop::handle_switch_agent`).
//!
//! This module owns ONLY the plan↔act state flip. There is no code path here
//! that can reach `UiCmd::SwitchAndStart` / `start_turn`, so a mode-switch
//! chord (ctrl+t / t+Tab) can never start execution regardless of
//! `plan_submitted` state: when idle it sends `UiCmd::SwitchAgent` (the
//! worker applies the mode at the next turn boundary; it never starts one),
//! and while busy it refuses with the busy hint.

use std::path::Path;

use tokio::sync::mpsc;

use crate::app_helpers::{mode_switch_busy_flash, sys_tokens_for};
use crate::chat::ChatView;
use crate::worker::{dedup_switch, try_send_idempotent, UiCmd};

/// Shared pure-switch send tail: mode flash + best-effort `try_send` (a full
/// cmd channel must never block the UI loop) + drop consecutive same-name
/// repeats. Used by the ctrl+t chord below and by the non-handoff branch of
/// `app_loop::handle_switch_agent` (Shift+Tab with no handoff armed).
pub(crate) fn pure_switch_send(
    name: &str,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    cmd_tx: &mpsc::Sender<UiCmd>,
    last_switch_sent: &mut Option<UiCmd>,
) {
    *mode_flash = Some((format!("\u{2192} {name} mode"), anim_tick));
    let next = UiCmd::SwitchAgent(name.to_string());
    if !dedup_switch(last_switch_sent.as_ref(), &next) && try_send_idempotent(cmd_tx, next.clone())
    {
        *last_switch_sent = Some(next);
    }
}

/// Handle the ctrl+t / t+Tab pure mode switch: a plan↔act state flip only,
/// NEVER execution. Bidirectional running gate — busy (a turn in flight OR a
/// live subagent) rejects with the busy hint: nothing is sent, agent/input/
/// running/sys_tokens stay untouched, and the user re-presses at a clean
/// idle boundary (no deferred auto-fire). When idle: the switch is folded
/// optimistically (status chip stays correct even if the AgentSwitch event
/// is dropped under channel pressure) and `UiCmd::SwitchAgent` is sent.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_pure_mode_switch(
    name: String,
    chat: &mut ChatView,
    running: bool,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    cmd_tx: &mpsc::Sender<UiCmd>,
    sys_tokens: &mut u64,
    workdir: &Path,
    active_skill_body: &Option<String>,
    last_switch_sent: &mut Option<UiCmd>,
) {
    if running || chat.subagents_running > 0 {
        // Busy: the worker is mid-run_session and a mid-turn mode flip would
        // apply at an arbitrary partial boundary. Intercept with an explicit
        // hint — the user re-presses at a clean idle boundary.
        *mode_flash = Some(mode_switch_busy_flash(anim_tick));
        return;
    }
    *sys_tokens = sys_tokens_for(&name, workdir, active_skill_body.as_deref());
    // Optimistic fold: correct chip under channel pressure, and a stale
    // `plan_submitted` collapses synchronously (rapid double-tap hygiene).
    // Execution semantics are intentionally absent here: see module docs.
    chat.fold_agent_switch(&name);
    pure_switch_send(&name, mode_flash, anim_tick, cmd_tx, last_switch_sent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn plan_submitted_view() -> ChatView {
        ChatView {
            agent: "plan".into(),
            plan_submitted: true,
            ..Default::default()
        }
    }

    fn harness() -> (mpsc::Sender<UiCmd>, mpsc::Receiver<UiCmd>) {
        mpsc::channel(64)
    }

    /// Structural separation end-to-end: with a SUBMITTED plan, ctrl+t must
    /// deliver exactly `UiCmd::SwitchAgent` — no `SwitchAndStart` can ever be
    /// produced by this module (no `start_turn` call exists on the path).
    #[test]
    fn pure_switch_sends_only_switch_agent_despite_submitted_plan() {
        let mut chat = plan_submitted_view();
        let (cmd_tx, mut cmd_rx) = harness();
        let mut mode_flash = None;
        let mut sys_tokens = 0u64;
        let mut last_sent = None;
        let skill: Option<String> = None;

        handle_pure_mode_switch(
            "act".into(),
            &mut chat,
            false,
            &mut mode_flash,
            0,
            &cmd_tx,
            &mut sys_tokens,
            Path::new("."),
            &skill,
            &mut last_sent,
        );

        assert_eq!(chat.agent, "act", "mode flips optimistically");
        match cmd_rx.try_recv().expect("SwitchAgent must be sent") {
            UiCmd::SwitchAgent(n) => assert_eq!(n, "act"),
            other => panic!("only SwitchAgent is representable here: {other:?}"),
        }
        assert!(cmd_rx.try_recv().is_err(), "nothing else — ever");
    }

    /// A live subagent with `running == false` (dropped SubagentEnd) still
    /// blocks the pure switch: the gate is bidirectional over BOTH busy
    /// signals, and sys_tokens/agent stay untouched.
    #[test]
    fn pure_switch_blocked_while_subagent_live_even_if_running_false() {
        let mut chat = ChatView {
            agent: "plan".into(),
            subagents_running: 1,
            plan_submitted: true,
            ..Default::default()
        };
        let (cmd_tx, mut cmd_rx) = harness();
        let mut mode_flash = None;
        let mut sys_tokens = 7u64;
        let skill: Option<String> = None;

        handle_pure_mode_switch(
            "act".into(),
            &mut chat,
            false,
            &mut mode_flash,
            0,
            &cmd_tx,
            &mut sys_tokens,
            Path::new("."),
            &skill,
            &mut None,
        );

        assert!(cmd_rx.try_recv().is_err(), "busy ⇒ nothing is sent");
        assert_eq!(chat.agent, "plan", "mode unchanged on the block");
        assert_eq!(sys_tokens, 7, "sys_tokens untouched on the block");
        assert!(mode_flash
            .as_ref()
            .is_some_and(|(t, _)| t.contains("busy") && t.contains("blocked")));
    }

    /// Consecutive same-name repeats are deduped (no channel spam on rapid
    /// double-tap); a different name is always sent.
    #[test]
    fn pure_switch_dedups_consecutive_same_name() {
        let mut chat = plan_submitted_view();
        let (cmd_tx, mut cmd_rx) = harness();
        let mut mode_flash = None;
        let mut sys_tokens = 0u64;
        let mut last_sent = None;
        let skill: Option<String> = None;

        for _ in 0..3 {
            handle_pure_mode_switch(
                "act".into(),
                &mut chat,
                false,
                &mut mode_flash,
                0,
                &cmd_tx,
                &mut sys_tokens,
                Path::new("."),
                &skill,
                &mut last_sent,
            );
        }
        assert_eq!(
            cmd_rx
                .try_recv()
                .map(|c| matches!(c, UiCmd::SwitchAgent(_))),
            Ok(true),
            "first switch is sent"
        );
        assert!(cmd_rx.try_recv().is_err(), "same-name repeats are deduped");

        handle_pure_mode_switch(
            "plan".into(),
            &mut chat,
            false,
            &mut mode_flash,
            0,
            &cmd_tx,
            &mut sys_tokens,
            Path::new("."),
            &skill,
            &mut last_sent,
        );
        assert!(
            matches!(cmd_rx.try_recv(), Ok(UiCmd::SwitchAgent(ref n)) if n == "plan"),
            "a different name is always sent"
        );
    }
}
