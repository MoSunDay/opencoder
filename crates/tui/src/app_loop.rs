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
use crate::keymap_menu::KeymapMenu;
use crate::local_cmd;
use crate::model_menu::{ConfigForm, ModelMenu, ProviderList};
use crate::task::TaskPicker;
use crate::theme;
use crate::worker::{gate_compact, gate_switch, CompactGate, SwitchGate, UiCmd, UiEvent};

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
/// titles and context stats that vary depending on whether a subagent
/// perspective is being viewed.
///
/// `display_chat` is a borrow into the live `ChatView` (either the parent's or a
/// subagent block's child view), matching the original inline code which held a
/// `&ChatView` rather than cloning.
pub(crate) struct DisplayState<'a> {
    pub(crate) agent_name: String,
    pub(crate) display_mode: String,
    pub(crate) status: String,
    pub(crate) display_chat: &'a ChatView,
    /// Body block title. Top level: `workdir · model · effort`; subagent
    /// focus: the back/navigation title.
    pub(crate) display_title: Line<'static>,
    pub(crate) display_ctx: u64,
    pub(crate) display_sys: u64,
}

/// Compute the per-iteration display values — `display_chat`, `display_title`,
/// `display_ctx` and `display_sys` — swapping in a subagent's child ChatView
/// when one is focused. The top-level title renders workdir, model name, and
/// thinking effort with the same plain style. The mode remains in the bottom
/// status bar. Pure: reads state, returns the values; the caller assigns them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_display<'a>(
    chat: &'a ChatView,
    subagent_focus: Option<usize>,
    subagent_sys: u64,
    sys_tokens: u64,
    config: &Config,
    workdir: &Path,
    _row_width: u16,
    _arrow_w: u16,
) -> DisplayState<'a> {
    let agent_name = chat.agent.clone();
    let status = chat.status.clone();
    // When viewing a subagent's perspective, swap in its child ChatView,
    // back-title, and its own context stats (instead of the parent's).
    // The body title keeps the "Ctrl+L back" hint.
    let (display_chat, display_title, display_ctx, display_sys, display_mode) = if let Some(idx) = subagent_focus
    {
        match chat.blocks.get(idx) {
            Some(crate::chat::ChatBlock::Subagent {
                view, kind, prompt, ..
            }) => (
                view as &crate::chat::ChatView,
                Line::from(format!(
                    "\u{2190} [Ctrl+L] back | \u{2937}sub [{kind}] {prompt}"
                )),
                view.context_used,
                subagent_sys,
                kind.clone(),
            ),
            _ => (
                chat,
                Line::from(agent_name.clone()),
                chat.context_used,
                sys_tokens,
                agent_name.clone(),
            ),
        }
    } else {
        let title = super::app_display::compose_top_title(
            workdir,
            config.model_id(),
            config.reasoning_effort.as_deref(),
        );
        (chat, title, chat.context_used, sys_tokens, agent_name.clone())
    };
    DisplayState {
        agent_name,
        display_mode,
        status,
        display_chat,
        display_title,
        display_ctx,
        display_sys,
    }
}

/// Advance the task-total clock using a single `last_clock` dt baseline.
/// Provider/model round timing is event-driven in `ChatView`; it is distinct
/// from this outer task clock because one running task can contain many rounds.
///
/// Order of operations (single shared `dt`/`last_clock` baseline):
/// - `false -> true`: snap the baseline so idle time is not counted.
/// - `true -> false`: freeze the task total.
/// - `running` stays `true`: accumulate `dt` into the task total.
pub(crate) fn tick_clock(
    running: bool,
    prev_running: &mut bool,
    last_clock: &mut Instant,
    task_elapsed_ms: &mut u64,
) {
    let now = Instant::now();
    if running && !*prev_running {
        // Task started: snap the baseline so the idle gap isn't counted.
        *last_clock = now;
    }
    *prev_running = running;
    let dt = now.duration_since(*last_clock).as_millis() as u64;
    *last_clock = now;
    if running {
        *task_elapsed_ms = task_elapsed_ms.saturating_add(dt);
    }
}

/// Outcome of [`handle_switch_agent`]: mirrors the `break` (quit) that lived
/// inline in the loop body when the worker channel died.
pub(crate) enum SwitchOutcome {
    Proceed,
    Quit,
}

/// Handle `KeyAction::SwitchAgent` (and `SwitchAgentNoClear`): switch agent
/// mode, and for plan→act with a submitted plan, handoff immediately when idle,
/// no-op when running. `no_handoff` (SwitchAgentNoClear / t+Tab chord) skips
/// the plan→act handoff entirely — transcript preserved in full — but still
/// honors the running-gate (deferred to the next clean idle boundary).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_switch_agent(
    name: String,
    no_handoff: bool,
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
    if *running {
        // A turn is in flight — any mode switch is a no-op. The worker is
        // mid-`run_session`; applying the switch now would start the next turn
        // with a stale agent at an arbitrary partial boundary. sys_tokens is NOT
        // updated here: the agent stays in its current mode, so the context
        // meter must keep the current-mode baseline. (Updating it to the target
        // mode's count would corrupt the meter for the remainder of the running
        // turn.) The same rule covers plan→act with a submitted plan, act→plan,
        // and plan→act without a submitted plan — all deferred to the next
        // clean idle boundary.
        *mode_flash = Some(("\u{23f3} busy \u{2014} switch when idle".into(), anim_tick));
        return SwitchOutcome::Proceed;
    }
    *sys_tokens = sys_tokens_for(&name, workdir, active_skill_body.as_deref());
    // Optimistically reflect the switch so the status chip is correct even if
    // AgentSwitch is dropped under channel pressure. Covers non-turning switches
    // (Alt+Tab) that emit no TurnDone to reconcile against.
    chat.agent = crate::terminal_text::sanitize_single_line(&name).into_owned();
    if !no_handoff && plan_to_act && chat.plan_submitted {
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
    // A collapsed, already-visible Thinking block does not need a repaint for
    // every appended reasoning token. The first delta that creates the block
    // *does* need one so users immediately see that the model is thinking.
    // Keep the whole coalesced batch renderable once any visible change occurs;
    // a later hidden reasoning delta must not mask an earlier visible event.
    let mut batch_needs_render = false;
    for ev in events {
        *skip_next_render = false;
        let mut hidden_reasoning_append = false;
        match ev {
            UiEvent::Session(sev) => {
                if let SessionEvent::TranscriptReset(msgs) = &sev {
                    let agent = chat.agent.clone();
                    let saved_plan_submitted = chat.plan_submitted;
                    let saved_pending_plan_arm = chat.pending_plan_arm;
                    let saved_requirement_text = chat.requirement_text.clone();
                    let saved_first_prompt = chat.first_prompt.clone();
                    *chat =
                        crate::session_ui::replay_into_chat(&agent, msgs, store, session_id).await;
                    chat.plan_submitted = saved_plan_submitted;
                    chat.pending_plan_arm = saved_pending_plan_arm;
                    chat.requirement_text = saved_requirement_text;
                    chat.first_prompt = saved_first_prompt;
                } else {
                    hidden_reasoning_append = matches!(sev, SessionEvent::ReasoningDelta(_))
                        && chat.last_open_thinking_collapsed();
                    chat.apply(&sev);
                }
                if let SessionEvent::QueueConsumed { seq, text } = &sev {
                    // Echo at consume time (prompt was visible in pending panel).
                    // Prefer the text carried by the event (robust against a
                    // saturated UI channel dropping the mirror update); fall
                    // back to the local mirror for old events without text.
                    let display = if !text.is_empty() {
                        text.clone()
                    } else {
                        queue_items
                            .iter()
                            .find(|(s, _)| s == seq)
                            .map(|(_, d)| d.clone())
                            .unwrap_or_default()
                    };
                    if !display.is_empty() {
                        chat.push_marker(Line::from(Span::styled(
                            format!("user: {display}"),
                            Style::default()
                                .fg(theme::warn_color())
                                .add_modifier(Modifier::BOLD),
                        )));
                        chat.push_marker(Line::from(""));
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
                                    .pending_inputs(session_id, opencoder_store::Delivery::Queue)
                                    .await
                                    .unwrap_or_default(),
                            );
                            chat.steer_items = crate::queue_panel::pending_mirror(
                                store
                                    .pending_inputs(session_id, opencoder_store::Delivery::Steer)
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
                // A dropped AgentSwitch("plan") would leave a stale
                // pending_plan_arm behind, spuriously re-arming plan_submitted
                // on a *later* plan-mode entry. The event channel is FIFO, so
                // an unconsumed arm at TurnDone(plan) means exactly that the
                // switch event was dropped — consume the arm against the
                // authoritative agent here (before `agent` is moved below).
                if agent == "plan" && chat.pending_plan_arm {
                    chat.plan_submitted = true;
                    chat.pending_plan_arm = false;
                }
                chat.agent = crate::terminal_text::sanitize_single_line(&agent).into_owned();
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
        batch_needs_render |= !hidden_reasoning_append;
        *skip_next_render = !batch_needs_render;
    }
    LoopFlow::Proceed
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
    _keymap_menu: &mut Option<KeymapMenu>,
    agent_name: &str,
    input: &mut String,
    cursor_idx: &mut usize,
    config: &mut Config,
    workdir: &Path,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    sys_tokens: &mut u64,
    plan_edit: &mut Option<crate::plan_edit::PlanEdit>,
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
        CommandOutcome::Dispatch(SlashAction::Fork) => {
            // Same parent-session list, but in fork mode: Enter clones the
            // highlighted session's context into a brand-new session.
            let sessions = store
                .list_sessions(&opencoder_store::SessionFilter::default())
                .await
                .unwrap_or_default();
            *task_picker = Some(TaskPicker::new_fork(sessions, session_id.to_string()));
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
        // RUNNING-GATE: while a turn is in flight (`running`), all three are
        // refused with a `[switch] busy — retry when idle` marker — a mode
        // switch mid-turn would start the next turn with a stale agent at an
        // arbitrary partial boundary (mirrors `/compact`'s SkipRunning).
        CommandOutcome::Dispatch(SlashAction::Act) => match gate_switch(*running) {
            SwitchGate::Run => {
                if chat.agent == "plan" && chat.plan_submitted {
                    let extra = prep_plan_to_act(
                        input, cursor_idx, sys_tokens, mode_flash, anim_tick, workdir,
                    );
                    if !start_turn(cmd_tx, cancel, UiCmd::SwitchAndStart("act".into(), extra)).await
                    {
                        worker_dead(chat);
                        return LoopFlow::Quit;
                    }
                } else if !start_turn(cmd_tx, cancel, UiCmd::Prompt("/act".into(), Vec::new()))
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
        },
        CommandOutcome::Dispatch(SlashAction::Plan) => match gate_switch(*running) {
            SwitchGate::Run => {
                if !start_turn(cmd_tx, cancel, UiCmd::Prompt("/plan".into(), Vec::new())).await {
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
        },
        CommandOutcome::Dispatch(SlashAction::ClearContext) => match gate_switch(*running) {
            SwitchGate::Run => {
                if chat.agent == "plan" && chat.plan_submitted {
                    let extra = prep_plan_to_act(
                        input, cursor_idx, sys_tokens, mode_flash, anim_tick, workdir,
                    );
                    if !start_turn(cmd_tx, cancel, UiCmd::SwitchAndStart("act".into(), extra)).await
                    {
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
            SwitchGate::SkipRunning => {
                chat.push_marker(Line::from(Span::styled(
                    "[switch] busy \u{2014} retry when idle",
                    Style::default().fg(theme::warn_color()),
                )));
            }
        },
        // Display-only commands: inspect / kill background bash, toggle
        // autopilot. Never start a turn and never reach session.messages —
        // the result is pushed as a purple marker. Work in any state
        // (idle + mid-turn).
        CommandOutcome::Dispatch(SlashAction::Requirement) => {
            crate::plan_edit::enter_requirement(
                plan_edit,
                chat.last_requirement_text().unwrap_or_default(),
            );
            *mode_flash = Some(("\u{2192} requirement".into(), anim_tick));
        }
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
            input.push(' '); // trailing space so args/Enter work immediately
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

/// Handle a key in plan/requirement-edit mode. On Exit, persists iff modified.
/// On Continue, the editor is put back. Returns Redraw.
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
            match pe.kind() {
                crate::plan_edit::EditKind::Plan => {
                    chat.update_plan_text(&text);
                    let _ = cmd_tx.send(crate::worker::UiCmd::EditPlan(text)).await; }
                crate::plan_edit::EditKind::Requirement => {
                    chat.update_requirement_text(&text);
                    let _ = cmd_tx.send(crate::worker::UiCmd::EditRequirement(text)).await; }
            }
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
#[cfg(test)]
pub(crate) use app_loop_model::env_model_override;

#[path = "app_loop_paste.rs"]
mod app_loop_paste;

pub(crate) use app_loop_paste::{paste_clipboard_image, paste_clipboard_image_silent, route_paste};

/// Handle a keystroke while the keymap-rebinding modal is open. On `Save`,
/// persists the changed keymap fields to disk, reloads config, and rebuilds
/// the `KeyBindings` so the new shortcuts take effect immediately. On `Quit`,
/// sends `UiCmd::Quit` and returns [`LoopFlow::Quit`].
pub(crate) async fn handle_keymap_outcome(
    keymap_menu: &mut Option<KeymapMenu>,
    k: KeyEvent,
    config: &mut Config,
    keymap: &mut crate::keymap::KeyBindings,
    workdir: &Path,
    cmd_tx: &mpsc::Sender<UiCmd>,
) -> LoopFlow {
    use crate::keymap_menu::{handle_keymap_key, KeymapOutcome};
    match handle_keymap_key(keymap_menu, k) {
        KeymapOutcome::Quit => {
            let _ = cmd_tx.send(UiCmd::Quit).await;
            LoopFlow::Quit
        }
        KeymapOutcome::Save(patch) => {
            if Config::save(workdir, &patch).is_ok() {
                if let Ok(new_config) = Config::load(workdir) {
                    *config = new_config;
                    *keymap = crate::keymap::KeyBindings::from_config(config);
                }
            }
            LoopFlow::Proceed
        }
        KeymapOutcome::Cancel | KeymapOutcome::Idle => LoopFlow::Proceed,
    }
}
