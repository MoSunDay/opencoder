//! Mode-switch slash-command dispatch (`/act`, `/plan`, `/act_clear_context`)
//! extracted from `app_loop.rs` to keep that file under the 800-line
//! iteration cap. The three commands share one gate-and-start flow: a
//! running/subagent busy gate, an optional plan→act handoff, and the plain
//! prompt fallback. This is a pure move — the logic, signatures and doc
//! comments are unchanged from their original inline location. The
//! `pub(crate)` items are re-exported from `app_loop.rs`, so the call sites
//! in `dispatch_command` stay thin.

use std::path::Path;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{prep_plan_to_act, LoopFlow};
use crate::app_helpers::{start_turn, worker_dead};
use crate::chat::ChatView;
use crate::theme;
use crate::worker::{gate_switch, SwitchGate, UiCmd};

/// Which mode-switch command triggered the dispatch. Parameterizes the plain
/// prompt text and whether the plan→act handoff applies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ModeSwitch {
    Act,
    Plan,
    ClearContext,
}

impl ModeSwitch {
    fn prompt(self) -> &'static str {
        match self {
            ModeSwitch::Act => "/act",
            ModeSwitch::Plan => "/plan",
            ModeSwitch::ClearContext => "/act_clear_context",
        }
    }

    /// `/act` and `/act_clear_context` from plan mode with a submitted plan
    /// route through `SwitchAndStart` (plan→act handoff) — same as Shift+Tab
    /// — preserving the plan and starting execution instead of wiping the
    /// transcript. `/plan` never hands off.
    fn may_handoff(self) -> bool {
        !matches!(self, ModeSwitch::Plan)
    }
}

/// Dispatch one of the three mode-switch slash commands through the worker.
/// `run_with_registry` short-circuits them (no LLM call) and emits
/// `AgentSwitch` / `TranscriptReset` + `Done`. No user echo — the popup path
/// never calls `push_user`.
///
/// RUNNING-GATE: while a turn is in flight (`running`), all three are refused
/// with a `[switch] busy — retry when idle` marker — a mode switch mid-turn
/// would start the next turn with a stale agent at an arbitrary partial
/// boundary (mirrors `/compact`'s `SkipRunning`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_mode_switch(
    mode: ModeSwitch,
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    running: &mut bool,
    follow: &mut bool,
    chat: &mut ChatView,
    input: &mut String,
    cursor_idx: &mut usize,
    sys_tokens: &mut u64,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    workdir: &Path,
) -> LoopFlow {
    match gate_switch(*running || chat.subagents_running > 0) {
        SwitchGate::Run => {
            if mode.may_handoff() && chat.agent == "plan" && chat.plan_submitted {
                let extra = prep_plan_to_act(
                    input, cursor_idx, sys_tokens, mode_flash, anim_tick, workdir,
                );
                if !start_turn(cmd_tx, cancel, UiCmd::SwitchAndStart("act".into(), extra)).await {
                    worker_dead(chat);
                    return LoopFlow::Quit;
                }
            } else if !start_turn(
                cmd_tx,
                cancel,
                UiCmd::Prompt(mode.prompt().into(), Vec::new()),
            )
            .await
            {
                worker_dead(chat);
                return LoopFlow::Quit;
            }
            *running = true;
            *follow = true;
            chat.begin_turn();
        }
        SwitchGate::SkipRunning => {
            chat.push_marker(Line::from(Span::styled(
                "[switch] busy \u{2014} retry when idle",
                Style::default().fg(theme::warn_color()),
            )));
        }
    }
    LoopFlow::Proceed
}
