//! Free-function helpers extracted from `app.rs`'s `run_app` event loop to keep
//! that file under the 800-line iteration cap. These mirror the `app_helpers`
//! extraction pattern: each is a `pub(crate)` free function taking `&mut` / `&`
//! references to the loop's locals, so the call sites in `app.rs` stay thin.
//!
//! Control-flow note: several extracted blocks used `continue` (re-render the
//! same loop turn) or `break` (quit the loop) inside `run_app`'s
//! `loop { tokio::select! { ... } }`. Those are translated into a returned
//! `LoopFlow` value that the caller maps back to `continue`/`break` — see the
//! call sites in `app.rs`.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::KeyEvent;
use opencoder_core::Config;
use opencoder_session::SessionEvent;
use opencoder_store::Store;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app_helpers::{start_turn, sys_tokens_for, worker_dead};
use crate::cache_salt_menu::CacheSaltMenu;
use crate::chat::ChatView;
use crate::command::{handle_command_key, CommandMenu, CommandOutcome, SlashAction};
use crate::local_cmd;
use crate::model_menu::{ConfigForm, ModelMenu, ProviderList};
use crate::task::TaskPicker;
use crate::theme;
use crate::worker::{gate_compact, CompactGate, UiCmd, UiEvent};

/// Translation of the `continue` / `break` control flow that lived inside the
/// extracted loop blocks. `Proceed` means fall through to the rest of the loop
/// body (the block did neither `continue` nor `break`); `Redraw` was a
/// `continue` (jump to the next turn, re-render); `Quit` was a `break`
/// (exit the loop).
pub(crate) enum LoopFlow {
    Proceed,
    /// Used by extracted blocks that previously did `continue` (re-render).
    Redraw,
    /// `/install_tools`: run the deps installer (handled one frame up in
    /// `run_app` since `dispatch_command` lacks the terminal handle).
    InstallTools,
    Quit,
}

/// Per-iteration display state computed by [`compute_display`]: the chat view,
/// titles, context stats and model label that vary depending on whether a
/// subagent perspective is being viewed.
///
/// `display_chat` is a borrow into the live `ChatView` (either the parent's or a
/// subagent block's child view), matching the original inline code which held a
/// `&ChatView` rather than cloning.
pub(crate) struct DisplayState<'a> {
    pub(crate) agent_name: String,
    pub(crate) status: String,
    pub(crate) display_chat: &'a ChatView,
    pub(crate) display_title: String,
    pub(crate) display_status_agent: String,
    pub(crate) display_ctx: u64,
    pub(crate) display_sys: u64,
    pub(crate) status_model: String,
}

/// Compute the per-iteration display values — `display_chat`, `display_title`,
/// `display_status_agent`, `display_ctx`, `display_sys` and `status_model` —
/// swapping in a subagent's child ChatView when one is focused. Pure: reads
/// state, returns the values; the caller assigns them into its locals.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_display<'a>(
    chat: &'a ChatView,
    subagent_focus: Option<usize>,
    subagent_sys: u64,
    sys_tokens: u64,
    config: &Config,
    workdir: &Path,
) -> DisplayState<'a> {
    let agent_name = chat.agent.clone();
    let status = chat.status.clone();
    // When viewing a subagent's perspective, swap in its child ChatView,
    // back-title, and its own context stats (instead of the parent's).
    // The body title keeps the "Ctrl+L back" hint; the status bar uses the
    // short subagent kind so it renders the same layout as the parent.
    let (display_chat, display_title, display_status_agent, display_ctx, display_sys) =
        if let Some(idx) = subagent_focus {
            match chat.blocks.get(idx) {
                Some(crate::chat::ChatBlock::Subagent {
                    view, kind, prompt, ..
                }) => (
                    view as &crate::chat::ChatView,
                    format!("\u{2190} [Ctrl+L] back | \u{2937}sub [{kind}] {prompt}"),
                    kind.clone(),
                    view.context_used,
                    subagent_sys,
                ),
                _ => (
                    chat,
                    agent_name.clone(),
                    agent_name.clone(),
                    chat.context_used,
                    sys_tokens,
                ),
            }
        } else {
            (
                chat,
                workdir.display().to_string(),
                agent_name.clone(),
                chat.context_used,
                sys_tokens,
            )
        };
    // Status bar shows the bare model id (without provider prefix) plus an
    // optional reasoning-effort badge, e.g. "glm-5.2 \u{00b7}high".
    let mid = config.model_id();
    let status_model = match &config.reasoning_effort {
        Some(e) if !e.trim().is_empty() => format!("{mid} \u{00b7}{e}"),
        _ => mid.to_string(),
    };
    DisplayState {
        agent_name,
        status,
        display_chat,
        display_title,
        display_status_agent,
        display_ctx,
        display_sys,
        status_model,
    }
}

/// Advance the status-bar run-timer: accumulates wall-clock elapsed time while
/// a turn is running. Called every loop iteration before the select.
pub(crate) fn tick_clock(running: bool, last_clock: &mut Instant, run_elapsed_ms: &mut u64) {
    let now = Instant::now();
    let dt = now.duration_since(*last_clock).as_millis() as u64;
    *last_clock = now;
    if running {
        *run_elapsed_ms = run_elapsed_ms.saturating_add(dt);
    }
}

/// Outcome of [`handle_switch_agent`]: mirrors the `break` (quit) that lived
/// inline in the loop body when the worker channel died.
pub(crate) enum SwitchOutcome {
    Proceed,
    Quit,
}

/// Handle `KeyAction::SwitchAgent`: switch agent mode, and for plan→act with a
/// submitted plan, handoff immediately when idle, no-op when running.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_switch_agent(
    name: String,
    chat: &mut ChatView,
    running: &mut bool,
    follow: &mut bool,
    input: &mut String,
    cursor_idx: &mut usize,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    sys_tokens: &mut u64,
    workdir: &Path,
    active_skill_body: &Option<String>,
) -> SwitchOutcome {
    let plan_to_act = chat.agent == "plan" && name == "act";
    if plan_to_act && chat.plan_submitted && *running {
        // Plan turn still running — Shift+Tab is a no-op. sys_tokens is NOT
        // updated here: the agent stays in plan mode, so the context meter must
        // keep the plan-mode baseline. (Updating it to the act-mode count would
        // corrupt the meter for the remainder of the running plan turn.)
        *mode_flash = Some(("\u{21bb} plan running\u{2026}".into(), anim_tick));
        return SwitchOutcome::Proceed;
    }
    *sys_tokens = sys_tokens_for(&name, workdir, active_skill_body.as_deref());
    // Optimistically reflect the switch so the status chip is correct even if
    // AgentSwitch is dropped under channel pressure. Covers non-turning switches
    // (Alt+Tab) that emit no TurnDone to reconcile against.
    chat.agent = name.clone();
    if plan_to_act && chat.plan_submitted {
        // Idle: handoff immediately, carrying any input text.
        let extra = std::mem::take(input);
        *cursor_idx = 0;
        *mode_flash = Some((format!("\u{2192} {name} mode"), anim_tick));
        if !start_turn(cmd_tx, cancel, UiCmd::SwitchAndStart(name, extra)).await {
            worker_dead(chat);
            return SwitchOutcome::Quit;
        }
        *running = true;
        *follow = true;
        chat.begin_turn();
    } else {
        *mode_flash = Some((format!("\u{2192} {name} mode"), anim_tick));
        let _ = cmd_tx.send(UiCmd::SwitchAgent(name)).await;
    }
    SwitchOutcome::Proceed
}

/// Shared plan→act handoff prep for the `/act` and `/act_clear_context` slash
/// commands (and mirrors the Shift+Tab path in [`handle_switch_agent`]):
/// drain the input box, refresh the context-meter baseline, set the mode-flash
/// banner. Returns the captured input text to forward as `SwitchAndStart`'s
/// extra payload.
fn prep_plan_to_act(
    input: &mut String,
    cursor_idx: &mut usize,
    sys_tokens: &mut u64,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    workdir: &Path,
) -> String {
    let extra = std::mem::take(input);
    *cursor_idx = 0;
    *sys_tokens = sys_tokens_for("act", workdir, None);
    *mode_flash = Some(("\u{2192} act mode".into(), anim_tick));
    extra
}

/// Body of the `maybe_ev = evt_rx.recv()` select arm: drain all queued
/// `UiEvent`s and fold them into the chat / queue state. Returns
/// [`LoopFlow::Quit`] when the worker channel closed (`recv()` gave `None`),
/// otherwise [`LoopFlow::Proceed`] (the caller then sets `dirty = true`).
///
/// `maybe_ev` is the value already produced by the select branch's `recv()`;
/// `evt_rx` is borrowed again to drain any further coalesced events via
/// `try_recv`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn fold_ui_events(
    maybe_ev: Option<UiEvent>,
    chat: &mut ChatView,
    store: &Arc<dyn Store>,
    session_id: &str,
    queue_items: &mut Vec<(i64, String)>,
    running: &mut bool,
    cancelled: &mut bool,
    drain_pending: &mut bool,
    skip_next_render: &mut bool,
    follow: &mut bool,
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    evt_rx: &mut mpsc::Receiver<UiEvent>,
) -> LoopFlow {
    let ev = match maybe_ev {
        Some(ev) => ev,
        None => {
            worker_dead(chat);
            return LoopFlow::Quit;
        }
    };
    // Drain all queued events to coalesce token bursts into one
    // batch — process them all now, render at most once next frame.
    let mut events = vec![ev];
    while let Ok(ev) = evt_rx.try_recv() {
        events.push(ev);
    }
    for ev in events {
        *skip_next_render = false;
        match ev {
            UiEvent::Session(sev) => {
                if let SessionEvent::TranscriptReset(msgs) = &sev {
                    let agent = chat.agent.clone();
                    let saved_plan_submitted = chat.plan_submitted;
                    *chat =
                        crate::session_ui::replay_into_chat(&agent, msgs, store, session_id).await;
                    chat.plan_submitted = saved_plan_submitted;
                } else {
                    chat.apply(&sev);
                    if matches!(sev, SessionEvent::ReasoningDelta(_))
                        && chat.last_thinking_collapsed()
                    {
                        *skip_next_render = true;
                    }
                }
                if let SessionEvent::QueueConsumed { seq } = &sev {
                    // Echo at consume time (prompt was visible in pending panel).
                    if let Some((_, d)) = queue_items.iter().find(|(s, _)| s == seq).cloned() {
                        chat.push_marker(Line::from(Span::styled(format!("queued: {d}"), Style::default().fg(theme::warn_color()).add_modifier(Modifier::BOLD))));
                    }
                    queue_items.retain(|(s, _)| s != seq);
                }
                if matches!(sev, SessionEvent::Done | SessionEvent::Error(_)) {
                    if *cancelled {
                        // Stale event from a cancelled turn — consume without
                        // affecting running or clearing items belonging to a
                        // potentially-new turn.
                        *cancelled = false;
                    } else if !*drain_pending {
                        if matches!(sev, SessionEvent::Done) {
                            // Re-sync both queue AND steer mirrors from the
                            // store. Under FIFO drain-to-empty semantics Done
                            // normally means everything is drained, but a
                            // cancel/interrupt/race can break the run early
                            // with inputs still pending. If any are found,
                            // arm drain_pending so TurnDone restarts the drain
                            // loop instead of going idle (which would strand
                            // the inputs permanently).
                            *queue_items = crate::queue_panel::pending_mirror(
                                store
                                    .pending_inputs(
                                        session_id,
                                        opencoder_store::Delivery::Queue,
                                    )
                                    .await
                                    .unwrap_or_default(),
                            );
                            chat.steer_items = crate::queue_panel::pending_mirror(
                                store
                                    .pending_inputs(
                                        session_id,
                                        opencoder_store::Delivery::Steer,
                                    )
                                    .await
                                    .unwrap_or_default(),
                            );
                            if !queue_items.is_empty() || !chat.steer_items.is_empty() {
                                // Stranded inputs found — arm drain_pending so
                                // the next TurnDone restarts the drain loop.
                                *drain_pending = true;
                            } else {
                                *running = false;
                            }
                        } else {
                            // Error: go idle without auto-restart to avoid
                            // error loops. Queue mirror is maintained per-item
                            // by QueueConsumed events as before.
                            *running = false;
                            chat.steer_items.clear();
                        }
                    }
                }
            }
            UiEvent::TurnDone(agent) => {
                // Reconcile the status chip from the authoritative agent.
                // AgentSwitch (the only other writer of chat.agent) is delivered
                // via try_send and may be dropped when the UI channel saturates.
                // TurnDone uses send().await (always lands).
                chat.agent = agent;
                // Safety net: SessionEvent::Done (which triggers
                // finalize_assistant -> markdown::render) is sent via
                // try_send and may be dropped during token bursts.
                // TurnDone is sent via blocking send().await so it
                // always arrives. finalize_assistant is idempotent
                // (the `!*done` guard), so re-calling when Done was
                // already processed is a no-op.
                chat.finalize_assistant();
                if *drain_pending {
                    // The cancelled turn has finished draining — restart
                    // the drain loop to promote pending steers.
                    *drain_pending = false;
                    *cancelled = false;
                    start_turn(cmd_tx, cancel, UiCmd::Prompt(String::new(), Vec::new())).await;
                    *running = true;
                    *follow = true;
                    chat.begin_turn();
                } else if *cancelled {
                    *cancelled = false;
                } else {
                    *running = false;
                }
            }
        }
    }
    LoopFlow::Proceed
}

/// The `match handle_model_key(...)` block from the `/config` modal: on
/// `Save(json)` persists config, reloads it, rebuilds the outer client / config
/// / model label / context limit / frame ticker, sends `ReloadConfig` and posts
/// a marker. `Cancel | Idle` does nothing. `Quit` sends `UiCmd::Quit` and was a
/// `break`. Returns [`LoopFlow::Quit`] for the `Quit` arm, otherwise
/// [`LoopFlow::Proceed`] (the caller keeps the post-match `continue` inline).
/// Detect whether an exported `OPENCODER_MODEL` silently overrode a `/model`
/// switch. `Config::load` runs `apply_env` on every load, so an exported
/// `OPENCODER_MODEL` re-pins `cfg.model` and reverts a just-saved menu switch
/// -- leaving the status bar showing an unexpected model with no feedback.
///
/// Pure (no env I/O) so it is unit-testable without flaky process-wide env.
/// Returns the env model value when an override occurred, else `None`.
pub(crate) fn env_model_override(
    intended_model: Option<&str>,
    effective_model: &str,
    env_model: Option<&str>,
) -> Option<String> {
    let intended = intended_model?;
    let env = env_model?.trim();
    if env.is_empty() {
        return None;
    }
    (effective_model != intended).then(|| env.to_string())
}

/// The `match outcome` block from the `/` command picker modal: dispatches the
/// chosen `SlashAction` (open task picker, model/config menus, compact,
/// cache-salt panel). `handle_command_key` also returns a `quit` flag which, if
/// set, sends `UiCmd::Quit` and was a `break`. Returns [`LoopFlow::Quit`] on any
/// break path (`quit`, or compact-with-dead-worker); otherwise
/// [`LoopFlow::Proceed`] (the caller keeps the post-match `continue` inline).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_command(
    command_menu: &mut Option<CommandMenu>,
    k: KeyEvent,
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    chat: &mut ChatView,
    running: &mut bool,
    follow: &mut bool,
    store: &Arc<dyn Store>,
    session_id: &str,
    task_picker: &mut Option<TaskPicker>,
    model_menu: &mut Option<ModelMenu>,
    cache_salt_menu: &mut Option<CacheSaltMenu>,
    agent_name: &str,
    input: &mut String,
    cursor_idx: &mut usize,
    config: &mut Config,
    workdir: &Path,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    sys_tokens: &mut u64,
) -> LoopFlow {
    let (outcome, quit) = handle_command_key(command_menu, k);
    if quit {
        let _ = cmd_tx.send(UiCmd::Quit).await;
        return LoopFlow::Quit;
    }
    match outcome {
        CommandOutcome::Dispatch(SlashAction::Task) => {
            // Parent sessions only: `/task` switches back to a parent
            // conversation, subagent children are not listed here.
            let sessions = store
                .list_sessions(&opencoder_store::SessionFilter::default())
                .await
                .unwrap_or_default();
            *task_picker = Some(TaskPicker::new(sessions, session_id.to_string()));
        }
        CommandOutcome::Dispatch(SlashAction::Model) => {
            *model_menu = Some(ModelMenu::List(ProviderList::new(config)));
        }
        CommandOutcome::Dispatch(SlashAction::Config) => {
            *model_menu = Some(ModelMenu::Config(ConfigForm::new(config)));
        }
        CommandOutcome::Dispatch(SlashAction::Compact) => match gate_compact(*running) {
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
        CommandOutcome::Dispatch(SlashAction::CacheSalt) => {
            let enabled = config.cache_salt == Some(true);
            *cache_salt_menu = Some(
                match CacheSaltMenu::build(store.as_ref(), session_id, agent_name, enabled).await {
                    Ok(m) => m,
                    Err(_) => CacheSaltMenu::parent_only(agent_name, session_id, enabled),
                },
            );
        }
        // Control commands (/act, /plan, /act_clear_context): dispatch as a
        // prompt via the worker. run_with_registry short-circuits them (no LLM
        // call) and emits AgentSwitch / TranscriptReset + Done. No user echo —
        // the popup path never calls push_user.  EXCEPTION: /act and
        // /act_clear_context from plan mode with a submitted plan route through
        // SwitchAndStart (plan→act handoff) — same as Shift+Tab — preserving
        // the plan and starting execution instead of wiping the transcript.
        CommandOutcome::Dispatch(SlashAction::Act) => {
            if chat.agent == "plan" && chat.plan_submitted && !*running {
                let extra = prep_plan_to_act(
                    input, cursor_idx, sys_tokens, mode_flash, anim_tick, workdir,
                );
                if !start_turn(cmd_tx, cancel, UiCmd::SwitchAndStart("act".into(), extra)).await {
                    worker_dead(chat);
                    return LoopFlow::Quit;
                }
            } else if !start_turn(cmd_tx, cancel, UiCmd::Prompt("/act".into(), Vec::new())).await {
                worker_dead(chat);
                return LoopFlow::Quit;
            }
            *running = true;
            *follow = true;
            chat.begin_turn();
        }
        CommandOutcome::Dispatch(SlashAction::Plan) => {
            if !start_turn(cmd_tx, cancel, UiCmd::Prompt("/plan".into(), Vec::new())).await {
                worker_dead(chat);
                return LoopFlow::Quit;
            }
            *running = true;
            *follow = true;
            chat.begin_turn();
        }
        CommandOutcome::Dispatch(SlashAction::ClearContext) => {
            if chat.agent == "plan" && chat.plan_submitted && !*running {
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
                UiCmd::Prompt("/act_clear_context".into(), Vec::new()),
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
        // Display-only commands: inspect / kill background bash, toggle
        // autopilot. Never start a turn and never reach session.messages —
        // the result is pushed as a purple marker. Work in any state
        // (idle + mid-turn).
        CommandOutcome::Dispatch(SlashAction::Ps) => {
            local_cmd::run("/ps", chat, config, cmd_tx, workdir).await;
        }
        CommandOutcome::Dispatch(SlashAction::Stop) => {
            local_cmd::run("/stop", chat, config, cmd_tx, workdir).await;
        }
        CommandOutcome::Dispatch(SlashAction::Ap) => {
            local_cmd::run("/ap", chat, config, cmd_tx, workdir).await;
        }
        // `/install_tools`: handled one frame up in `run_app` (needs the
        // terminal handle to suspend/resume the screen). Decision only.
        CommandOutcome::Dispatch(SlashAction::InstallTools) => {
            return LoopFlow::InstallTools;
        }
        CommandOutcome::FillInput(s) => {
            input.clear();
            input.push_str(&s);
            input.push(' ');  // trailing space so args/Enter work immediately
            *cursor_idx = input.len();
            return LoopFlow::Redraw;
        }
        CommandOutcome::Idle => {}
    }
    LoopFlow::Proceed
}

/// Hard exit (Ctrl+C/Ctrl+D): interrupt any in-flight turn so the worker stops
/// promptly. Without cancelling the shared token the worker stays blocked inside
/// `run_session` and cannot read `UiCmd::Quit` until the turn naturally ends (up
/// to the 30-min timeout), freezing the terminal on the alt-screen.
pub(crate) async fn handle_quit(
    running: bool,
    cancel: &CancellationToken,
    chat: &mut ChatView,
    cmd_tx: &mpsc::Sender<UiCmd>,
) {
    if running {
        cancel.cancel();
        chat.push_marker(Line::from(Span::styled(
            "[exiting…]",
            Style::default().fg(theme::warn_color()),
        )));
    }
    let _ = cmd_tx.send(UiCmd::Quit).await;
}

/// Handle a key while in plan-edit mode. Takes ownership of the `Option<PlanEdit>`
/// via `take()` so there are no borrow conflicts. On `Exit`:
/// - If the text was modified, update the `ChatView` and send `UiCmd::EditPlan`.
/// - The `Option` stays `None` (plan editing ended).
///
/// On `Continue`: the `PlanEdit` is put back.
/// Returns [`LoopFlow::Redraw`] so the caller re-renders.
pub(crate) async fn handle_plan_edit_key(
    plan_edit: &mut Option<crate::plan_edit::PlanEdit>,
    k: crossterm::event::KeyEvent,
    chat: &mut crate::chat::ChatView,
    cmd_tx: &mpsc::Sender<crate::worker::UiCmd>,
    inner_w: u16,
) -> LoopFlow {
    let mut pe = match plan_edit.take() {
        Some(pe) => pe,
        None => return LoopFlow::Proceed,
    };
    if matches!(
        crate::plan_edit::handle_plan_edit_key(&mut pe, k, inner_w, 2),
        crate::plan_edit::PlanEditAction::Exit
    ) {
        if pe.is_modified() {
            let text = pe.text().to_string();
            chat.update_plan_text(&text);
            let _ = cmd_tx.send(crate::worker::UiCmd::EditPlan(text)).await;
        }
        // plan_edit stays None — editing ended
    } else {
        *plan_edit = Some(pe);
    }
    LoopFlow::Redraw
}

pub(crate) use crate::frame::render_frame;

/// Activate plan-edit mode using the text from the last Plan (or non-empty
/// Assistant) block, flashing the save/discard hint.
pub(crate) fn enter_plan_edit(
    plan_edit: &mut Option<crate::plan_edit::PlanEdit>,
    chat: &crate::chat::ChatView,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
) {
    if let Some(text) = chat.last_plan_text() {
        *plan_edit = Some(crate::plan_edit::PlanEdit::new(text));
        *mode_flash = Some(("\u{2192} plan mode".into(), anim_tick));
    }
}

/// Dispatch a key to the active plan-edit modal: compute the usable inner
/// width from the terminal, then delegate to [`handle_plan_edit_key`].
pub(crate) async fn dispatch_plan_edit_key(
    plan_edit: &mut Option<crate::plan_edit::PlanEdit>,
    k: KeyEvent,
    chat: &mut crate::chat::ChatView,
    cmd_tx: &mpsc::Sender<UiCmd>,
    terminal: &crate::render::Term,
) -> LoopFlow {
    let inner_w = terminal
        .size()
        .map(|r| r.width.saturating_sub(2))
        .unwrap_or(78);
    handle_plan_edit_key(plan_edit, k, chat, cmd_tx, inner_w).await
}

#[cfg(test)]
#[path = "app_loop_tests/mod.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "app_loop_bugfix_tests.rs"]
mod bugfix_tests;

#[path = "app_loop_model.rs"]
mod app_loop_model;

pub(crate) use app_loop_model::handle_model_outcome;

#[path = "app_loop_paste.rs"]
mod app_loop_paste;

pub(crate) use app_loop_paste::{paste_clipboard_image, paste_clipboard_image_silent, route_paste};
