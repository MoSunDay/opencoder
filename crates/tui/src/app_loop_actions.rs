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
use std::sync::Arc;

use opencoder_core::Config;
use opencoder_store::Store;

use crate::cache_salt_menu::CacheSaltMenu;
use crate::command::SlashAction;
use crate::local_cmd;
use crate::model_menu::{ConfigForm, ModelMenu, ProviderList};
use crate::worker::{gate_compact, gate_switch, CompactGate, SwitchGate, UiCmd};

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


/// Unified slash-command dispatch shared by both the `/` popup picker
/// (`dispatch_command`) and free-text composer submit (`app.rs` Submit).
/// Every recognized `/cmd` routes through here so the behavior is identical
/// regardless of entry path. Takes a [`SlashAction`] directly (the popup
/// extracts it from [`CommandOutcome`]; free-text uses [`command::parse`]).
///
/// Returns [`LoopFlow::Proceed`] for commands that only open a menu or
/// render chrome (Task, Fork, Model, Config, Mcp, CacheSalt, Annotation,
/// Notepad, Ps, Stop, Ap). Returns [`LoopFlow::InstallTools`] for
/// `/install_tools` (handled one frame up). For mode-switch commands
/// (Act, Plan, ClearContext, Compact) returns whatever the gate-and-start
/// flow yields (typically [`LoopFlow::Proceed`] or [`LoopFlow::Quit`]).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_slash_action(
    action: SlashAction,
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    chat: &mut ChatView,
    running: &mut bool,
    follow: &mut bool,
    store: &Arc<dyn Store>,
    session_id: &str,
    task_picker: &mut Option<crate::task::TaskPicker>,
    model_menu: &mut Option<ModelMenu>,
    mcp_menu: &mut Option<crate::mcp_menu::McpMenu>,
    cache_salt_menu: &mut Option<CacheSaltMenu>,
    agent_name: &str,
    input: &mut String,
    cursor_idx: &mut usize,
    config: &mut Config,
    workdir: &Path,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    sys_tokens: &mut u64,
    plan_edit: &mut Option<crate::plan_edit::PlanEdit>,
    notepad: &mut Option<crate::notepad::NotepadView>,
) -> LoopFlow {
    match action {
        SlashAction::Task => {
            let sessions = store
                .list_sessions(&opencoder_store::SessionFilter::default())
                .await
                .unwrap_or_default();
            *task_picker = Some(crate::task::TaskPicker::new(sessions, session_id.to_string()));
        }
        SlashAction::Fork => {
            let sessions = store
                .list_sessions(&opencoder_store::SessionFilter::default())
                .await
                .unwrap_or_default();
            *task_picker = Some(crate::task::TaskPicker::new_fork(sessions, session_id.to_string()));
        }
        SlashAction::Model => {
            *model_menu = Some(ModelMenu::List(ProviderList::new(config)));
        }
        SlashAction::Config => {
            *model_menu = Some(ModelMenu::Config(ConfigForm::new(config)));
        }
        SlashAction::Mcp => {
            *mcp_menu = Some(crate::mcp_menu::McpMenu::List(
                crate::mcp_menu::McpList::new(config),
            ));
        }
        SlashAction::Compact => match gate_compact(*running) {
            CompactGate::Run => {
                if !start_turn(cmd_tx, cancel, UiCmd::Compact).await {
                    worker_dead(chat);
                    return LoopFlow::Quit;
                }
                *running = true;
                *follow = true;
                chat.begin_turn();
            }
            CompactGate::SkipRunning => {
                chat.push_marker(Line::from(Span::styled(
                    "[compact] busy \u{2014} retry when idle",
                    Style::default().fg(theme::warn_color()),
                )));
            }
        },
        SlashAction::CacheSalt => {
            let enabled = config.cache_salt == Some(true);
            *cache_salt_menu = Some(
                match CacheSaltMenu::build(store.as_ref(), session_id, agent_name, enabled).await {
                    Ok(m) => m,
                    Err(_) => CacheSaltMenu::parent_only(agent_name, session_id, enabled),
                },
            );
        }
        SlashAction::Act => {
            return dispatch_mode_switch(
                ModeSwitch::Act, cmd_tx, cancel, running, follow, chat,
                input, cursor_idx, sys_tokens, mode_flash, anim_tick, workdir,
            ).await;
        }
        SlashAction::Plan => {
            return dispatch_mode_switch(
                ModeSwitch::Plan, cmd_tx, cancel, running, follow, chat,
                input, cursor_idx, sys_tokens, mode_flash, anim_tick, workdir,
            ).await;
        }
        SlashAction::ClearContext => {
            return dispatch_mode_switch(
                ModeSwitch::ClearContext, cmd_tx, cancel, running, follow, chat,
                input, cursor_idx, sys_tokens, mode_flash, anim_tick, workdir,
            ).await;
        }
        SlashAction::Annotation => {
            crate::plan_edit::enter_annotation(
                plan_edit,
                chat.last_annotation_text().unwrap_or_default(),
            );
            *mode_flash = Some(("\u{2192} annotation".into(), anim_tick));
        }
        SlashAction::Notepad => {
            *notepad = Some(crate::notepad::NotepadView::new(workdir.to_path_buf()));
        }
        SlashAction::Ps => {
            local_cmd::run("/ps", chat, config, cmd_tx, workdir).await;
        }
        SlashAction::Stop => {
            local_cmd::run("/stop", chat, config, cmd_tx, workdir).await;
        }
        SlashAction::Ap => {
            local_cmd::run("/ap", chat, config, cmd_tx, workdir).await;
        }
        // `/install_tools`: handled one frame up in `run_app` (needs the
        // terminal handle to suspend/resume the screen). Decision only.
        SlashAction::InstallTools => {
            return LoopFlow::InstallTools;
        }
    }
    LoopFlow::Proceed
}