//! Agent-switch slash-command dispatch (`/act`, `/sandbox`,
//! `/act_clear_context`) extracted from `app_loop.rs` to keep that file under
//! the 800-line iteration cap. The three commands share one gate-and-start
//! flow: a running/subagent busy gate, then submission of the control-command
//! text as a pure prompt. This is a pure move — the logic, signatures and doc
//! comments are unchanged from their original inline location. The
//! `pub(crate)` items are re-exported from `app_loop.rs`, so the call sites
//! in `dispatch_command` stay thin.

use std::path::Path;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::LoopFlow;
use crate::app_helpers::{start_turn, sys_tokens_for, worker_dead};
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

/// Which agent-switch command triggered the dispatch. Parameterizes the
/// control-command prompt text submitted to the runner.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ModeSwitch {
    Act,
    Sandbox,
    ClearContext,
}

impl ModeSwitch {
    fn prompt(self) -> &'static str {
        match self {
            ModeSwitch::Act => "/act",
            ModeSwitch::Sandbox => "/sandbox",
            ModeSwitch::ClearContext => "/clear_context",
        }
    }
}

/// Dispatch one of the three agent-switch slash commands through the worker.
/// `run_with_registry` short-circuits them (no LLM call) and emits
/// `AgentSwitch` / `TranscriptReset` + `Done`. No user echo — the popup path
/// never calls `push_user`.
///
/// RUNNING-GATE: while a turn is in flight (`running`), all three are refused
/// with a `[switch] busy — retry when idle` marker — an agent switch mid-turn
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
    sys_tokens: &mut u64,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    workdir: &Path,
) -> LoopFlow {
    match gate_switch(*running || chat.subagents_running > 0) {
        SwitchGate::Run => {
            let name = mode.prompt().trim_start_matches('/');
            *sys_tokens = sys_tokens_for(name, workdir, None);
            *mode_flash = Some((format!("\u{2192} {name} mode"), anim_tick));
            if !start_turn(
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
/// Notepad, Ps, Stop, Ap). For mode-switch commands
/// (Act, Sandbox, ClearContext, Compact) returns whatever the gate-and-start
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
    envs_menu: &mut Option<crate::envs_menu::EnvsMenu>,
    cli_menu: &mut Option<crate::cli_menu::CliMenu>,
    skill_toggle_menu: &mut Option<crate::skill_menu::SkillMenu>,
    ap_menu: &mut Option<crate::ap_menu::ApMenu>,
    cache_salt_menu: &mut Option<CacheSaltMenu>,
    agent_name: &str,
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
            *task_picker = Some(crate::task::TaskPicker::new(
                sessions,
                session_id.to_string(),
            ));
        }
        SlashAction::Fork => {
            let sessions = store
                .list_sessions(&opencoder_store::SessionFilter::default())
                .await
                .unwrap_or_default();
            *task_picker = Some(crate::task::TaskPicker::new_fork(
                sessions,
                session_id.to_string(),
            ));
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
        SlashAction::Envs => {
            *envs_menu = Some(crate::envs_menu::EnvsMenu::List(
                crate::envs_menu::EnvsList::discover(),
            ));
        }
        SlashAction::Cli => {
            *cli_menu = Some(crate::cli_menu::CliMenu::List(
                crate::cli_menu::CliList::new(config),
            ));
        }
        SlashAction::Skill => {
            *skill_toggle_menu = Some(crate::skill_menu::SkillMenu::List(
                crate::skill_menu::SkillList::new(config),
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
                ModeSwitch::Act,
                cmd_tx,
                cancel,
                running,
                follow,
                chat,
                sys_tokens,
                mode_flash,
                anim_tick,
                workdir,
            )
            .await;
        }
        SlashAction::Sandbox => {
            return dispatch_mode_switch(
                ModeSwitch::Sandbox,
                cmd_tx,
                cancel,
                running,
                follow,
                chat,
                sys_tokens,
                mode_flash,
                anim_tick,
                workdir,
            )
            .await;
        }
        SlashAction::ClearContext => {
            return dispatch_mode_switch(
                ModeSwitch::ClearContext,
                cmd_tx,
                cancel,
                running,
                follow,
                chat,
                sys_tokens,
                mode_flash,
                anim_tick,
                workdir,
            )
            .await;
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
            local_cmd::run("/ps", chat).await;
        }
        SlashAction::Stop => {
            local_cmd::run("/stop", chat).await;
        }
        SlashAction::Ap => {
            *ap_menu = Some(crate::ap_menu::ApMenu::new(config));
        }
    }
    LoopFlow::Proceed
}

/// Finish a queue-panel "submit now" (✎/>) mouse click: run the steer
/// decision and start a turn when it says so (extracted from `app.rs`'s
/// mouse arm to keep that file under the 800-line iteration cap).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn steer_submit_after_mouse(
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    subagent_focus: Option<usize>,
    running: &mut bool,
    chat: &mut crate::chat::ChatView,
    follow: &mut bool,
    child_runtime: &crate::worker::ChildRuntimeHandles,
    turn_cancel: &std::sync::Arc<std::sync::Mutex<CancellationToken>>,
) {
    let outcome = crate::app::steer_fire::handle_steer_submit(
        subagent_focus,
        *running,
        &child_runtime.cancels,
        &child_runtime.turn_cancels,
        turn_cancel,
        chat,
    );
    if outcome == crate::app::steer_fire::SteerSubmitOutcome::StartTurn {
        crate::app_helpers::start_turn(cmd_tx, cancel, UiCmd::Prompt(String::new(), Vec::new()))
            .await;
        *running = true;
        chat.begin_turn();
    }
    *follow = true;
}

/// Hard-cancel the running turn (double-Esc / Ctrl+C arm in `app.rs`):
/// cancel tokens and show the interrupted marker. Pending steer/queue rows
/// are deliberately KEPT — both in the
/// store and in the UI mirrors — so they are consumed FIFO on the next
/// submit's drain or a `>` panel drain. This matches the web
/// `/interrupt` semantics, where cancelling a run also preserves queued
/// input. We deliberately do NOT auto-restart the drain after cancel: the
/// user just explicitly cancelled, so silently resuming work would be
/// counterintuitive. Extracted to keep `app.rs` under the line cap.
pub(crate) async fn cancel_running_turn(
    chat: &mut crate::chat::ChatView,
    cancel: &mut CancellationToken,
    child_runtime: &mut crate::worker::ChildRuntimeHandles,
    running: &mut bool,
    cancelled: &mut bool,
    follow: &mut bool,
) {
    cancel.cancel();
    opencoder_session::fire_child_cancels(&child_runtime.cancels);
    chat.push_marker(ratatui::text::Line::from(ratatui::text::Span::styled(
        "[interrupted] stopping\u{2026}",
        ratatui::style::Style::default().fg(crate::theme::warn_color()),
    )));
    *running = false;
    *cancelled = true;
    *follow = true;
}
