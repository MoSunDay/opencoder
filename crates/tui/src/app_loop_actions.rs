//! Agent-switch slash-command dispatch (`/act`, `/plan`,
//! `/act_clear_context`) extracted from `app_loop.rs` to keep that file under
//! the 800-line iteration cap. The three commands share one gate-and-start
//! flow: a running/subagent busy gate, then submission of the control-command
//! text as a pure prompt. This is a pure move — the logic, signatures and doc
//! comments are unchanged from their original inline location. The
//! `pub(crate)` items are re-exported from `app_loop.rs`, so the call sites
//! in `dispatch_command` stay thin.

use crossterm::event::KeyEvent;
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
/// control-command prompt text submitted to the runner. The clear-context
/// fold is NOT here: it is destructive, so it goes through the
/// `clear_confirm` countdown guard (arm -> fire/cancel) instead.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ModeSwitch {
    Act,
    Plan,
}

impl ModeSwitch {
    pub(crate) fn for_agent(agent: &str) -> Self {
        if agent == "act" {
            Self::Act
        } else {
            Self::Plan
        }
    }

    fn prompt(self) -> &'static str {
        match self {
            ModeSwitch::Act => "/act",
            ModeSwitch::Plan => "/plan",
        }
    }
}

/// Dispatch an act/plan switch command through the worker.
/// `run_with_registry` short-circuits them (no LLM call) and emits
/// `AgentSwitch` / `TranscriptReset` + `Done`. No user echo — the popup path
/// never calls `push_user`.
///
/// SUBMIT-ALWAYS / APPLY-AT-IDLE (steer/queue semantics — mirrors the
/// `fire_clear_confirm` running arm): the switch can always be submitted,
/// but it only TAKES EFFECT at a non-running boundary. Idle starts the
/// control-command turn now; while a turn is in flight (`running`) the raw
/// command text queues verbatim and the runner applies it via the idle-boundary
/// drain intercept — a mid-turn switch never lands at an arbitrary partial
/// boundary, and the keystroke is never lost. A live subagent does not count
/// as busy: the parent session is idle, exactly when steer/queue entries are
/// consumed automatically.
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
    session_id: &str,
    admit_tx: &mpsc::Sender<crate::queue_admitter::AdmitReq>,
    admit_st: &mut crate::queue_admitter::AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    history: &mut Vec<String>,
    hist_idx: &mut Option<usize>,
) -> LoopFlow {
    match gate_switch(*running) {
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
            // Queue the raw command text (steer/queue semantics: submit now,
            // runner applies it at the idle boundary). No sys_tokens/mode
            // flash here — the switch has not landed yet; the AgentSwitch
            // event folds it when the runner consumes the row. Same running
            // arm shape as `fire_clear_confirm`.
            crate::queue_admitter::handle_queue(
                mode.prompt(),
                admit_tx,
                admit_st,
                queue_items,
                pending_images,
                session_id,
            );
            crate::app_helpers::push_history(history, hist_idx, mode.prompt());
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
/// Notepad, Ps, Stop, Ap, Sidecar). For mode-switch commands
/// (Act, Plan, ClearContext, Compact) returns whatever the gate-and-start
/// flow yields (typically [`LoopFlow::Proceed`] or [`LoopFlow::Quit`]). While
/// a turn is running, Act/Plan queue through `admit_tx` (apply at the idle
/// boundary) instead of starting a turn — see `dispatch_mode_switch`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_slash_action(
    action: SlashAction,
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    chat: &mut ChatView,
    sidecar_ask: &mpsc::Sender<crate::sidecar_ui::SidecarCmd>,
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
    clear_confirm: &mut Option<crate::clear_confirm::ClearConfirm>,
    admit_tx: &mpsc::Sender<crate::queue_admitter::AdmitReq>,
    admit_st: &mut crate::queue_admitter::AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    history: &mut Vec<String>,
    hist_idx: &mut Option<usize>,
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
                session_id,
                admit_tx,
                admit_st,
                queue_items,
                pending_images,
                history,
                hist_idx,
            )
            .await;
        }
        SlashAction::Plan => {
            return dispatch_mode_switch(
                ModeSwitch::Plan,
                cmd_tx,
                cancel,
                running,
                follow,
                chat,
                sys_tokens,
                mode_flash,
                anim_tick,
                workdir,
                session_id,
                admit_tx,
                admit_st,
                queue_items,
                pending_images,
                history,
                hist_idx,
            )
            .await;
        }
        SlashAction::ClearContext => {
            // Destructive fold: arm the countdown guard instead of firing.
            // The chip counts down; Esc cancels, Enter / window-elapsed fires
            // via `fire_clear_confirm`.
            crate::clear_confirm::engage(clear_confirm, chat, mode_flash, anim_tick, None, None);
            return LoopFlow::Proceed;
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
        SlashAction::Sidecar => {
            // Bypass semantics: the panel opens even mid-turn — the sidecar
            // never touches the parent's steer/queue/prompt paths, so the
            // running gate does not apply. Entry destroys the previous
            // conversation, so the next question sees a fresh snapshot.
            crate::sidecar_ui::enter_panel(chat, sidecar_ask);
            *follow = true;
            *mode_flash = Some((
                crate::sidecar_ui::SIDECAR_ENTER_FLASH.to_string(),
                anim_tick,
            ));
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

/// Fire an armed clear-context confirm. While running the compound command
/// text queues verbatim (the runner applies it at the idle boundary, tail
/// included); idle starts the control-command turn now, mirroring the
/// `dispatch_mode_switch` Run arm.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn fire_clear_confirm(
    cc: crate::clear_confirm::ClearConfirm,
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    running: &mut bool,
    follow: &mut bool,
    chat: &mut ChatView,
    sys_tokens: &mut u64,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    workdir: &Path,
    admit_tx: &mpsc::Sender<crate::queue_admitter::AdmitReq>,
    admit_st: &mut crate::queue_admitter::AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    session_id: &str,
    history: &mut Vec<String>,
    hist_idx: &mut Option<usize>,
) -> LoopFlow {
    let text = crate::clear_confirm::command_text(&cc);
    if *running {
        crate::queue_admitter::handle_queue(
            &text,
            admit_tx,
            admit_st,
            queue_items,
            pending_images,
            session_id,
        );
        crate::app_helpers::push_history(history, hist_idx, &text);
        return LoopFlow::Proceed;
    }
    // Echo the model-facing tail: the compound rest runs as a real user
    // turn in the fresh context, so it must be echoed; the command token
    // itself never is (applied inline, never recorded).
    if let Some(echo) = opencoder_session::consumed_echo_text(&text) {
        chat.blocks.push(crate::chat::ChatBlock::User {
            rendered: crate::markdown::render(&echo),
        });
        chat.push_marker(Line::from(""));
    }
    let name = crate::clear_confirm::CLEAR_CONTEXT_CMD.trim_start_matches('/');
    *sys_tokens = sys_tokens_for(name, workdir, None);
    *mode_flash = Some((format!("\u{2192} {name} mode"), anim_tick));
    if !start_turn(cmd_tx, cancel, UiCmd::Prompt(text, Vec::new())).await {
        worker_dead(chat);
        return LoopFlow::Quit;
    }
    *running = true;
    *follow = true;
    chat.begin_turn();
    LoopFlow::Proceed
}

/// Armed-guard key handling with all side effects: fire early on Enter
/// (`fire_clear_confirm`) — the composer text typed during the live window
/// merges into the compound rest first, so a submission executes right away
/// with what was typed — cancel on Esc (回撤 marker + draft restore), let
/// plain editing keys through. Returns true when the worker died firing
/// (caller quits).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_confirm_key(
    clear_confirm: &mut Option<crate::clear_confirm::ClearConfirm>,
    k: KeyEvent,
    input: &mut String,
    cursor_idx: &mut usize,
    undo_state: &mut crate::undo::UndoState,
    chat: &mut ChatView,
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    running: &mut bool,
    follow: &mut bool,
    sys_tokens: &mut u64,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    workdir: &Path,
    admit_tx: &mpsc::Sender<crate::queue_admitter::AdmitReq>,
    admit_st: &mut crate::queue_admitter::AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    session_id: &str,
    history: &mut Vec<String>,
    hist_idx: &mut Option<usize>,
) -> bool {
    match crate::clear_confirm::intercept(clear_confirm, input, cursor_idx, undo_state, k) {
        Some(crate::clear_confirm::ConfirmFlow::Fire) => {
            if let Some(mut cc) = clear_confirm.take() {
                // The submission confirms the countdown: fold the text typed
                // during the window into the compound rest and fire now
                // instead of waiting out the window.
                crate::clear_confirm::merge_typed(&mut cc, input);
                input.clear();
                *cursor_idx = 0;
                crate::undo::reset(undo_state, input, 0);
                return matches!(
                    fire_clear_confirm(
                        cc,
                        cmd_tx,
                        cancel,
                        running,
                        follow,
                        chat,
                        sys_tokens,
                        mode_flash,
                        anim_tick,
                        workdir,
                        admit_tx,
                        admit_st,
                        queue_items,
                        pending_images,
                        session_id,
                        history,
                        hist_idx,
                    )
                    .await,
                    LoopFlow::Quit
                );
            }
            false
        }
        Some(crate::clear_confirm::ConfirmFlow::Cancel) => {
            crate::clear_confirm::push_cancel_marker(chat);
            // Idle freezes anim_tick once the guard is gone, so a leftover
            // flash would stay pinned on screen forever — drop the chip.
            *mode_flash = None;
            false
        }
        None => false,
    }
}

/// Anim-tick side of the armed guard: refresh the countdown chip and
/// auto-fire when the window elapsed. Returns true when the worker died
/// firing (caller quits).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn confirm_tick(
    clear_confirm: &mut Option<crate::clear_confirm::ClearConfirm>,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    running: &mut bool,
    follow: &mut bool,
    chat: &mut ChatView,
    sys_tokens: &mut u64,
    workdir: &Path,
    admit_tx: &mpsc::Sender<crate::queue_admitter::AdmitReq>,
    admit_st: &mut crate::queue_admitter::AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    session_id: &str,
    history: &mut Vec<String>,
    hist_idx: &mut Option<usize>,
) -> bool {
    if let Some(cc) = crate::clear_confirm::tick(clear_confirm, mode_flash, anim_tick) {
        return matches!(
            fire_clear_confirm(
                cc,
                cmd_tx,
                cancel,
                running,
                follow,
                chat,
                sys_tokens,
                mode_flash,
                anim_tick,
                workdir,
                admit_tx,
                admit_st,
                queue_items,
                pending_images,
                session_id,
                history,
                hist_idx,
            )
            .await,
            LoopFlow::Quit
        );
    }
    false
}
