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
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app_helpers::{start_turn, sys_tokens_for, worker_dead};
use crate::cache_salt_menu::CacheSaltMenu;
use crate::chat::ChatView;
use crate::command::{handle_command_key, CommandMenu, CommandOutcome};
use crate::keymap_menu::KeymapMenu;
use crate::model_menu::ModelMenu;
use crate::task::TaskPicker;
use crate::theme;
use crate::worker::{UiCmd, UiEvent};

/// Translation of the `continue` / `break` control flow that lived inside the
/// extracted loop blocks. `Proceed` means fall through to the rest of the loop
/// body (the block did neither `continue` nor `break`); `Redraw` was a
/// `continue` (jump to the next turn, re-render); `Quit` was a `break`
/// (exit the loop).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoopFlow {
    Proceed,
    /// Used by extracted blocks that previously did `continue` (re-render).
    Redraw,
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
    let (display_chat, display_title, display_ctx, display_sys, display_mode) =
        if let Some(idx) = subagent_focus {
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
            (
                chat,
                title,
                chat.context_used,
                sys_tokens,
                agent_name.clone(),
            )
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
/// mode behind a direction-aware running gate.
///
/// plan→act while running (a turn in flight OR a live subagent) is
/// **intercepted with an explicit busy hint** — no deferred auto-fire; the
/// user re-presses at a clean idle boundary, where a submitted plan hands
/// off immediately (transcript fold + immediate execution, carrying any
/// input text). `no_handoff` (SwitchAgentNoClear / t+Tab chord) skips the
/// plan→act handoff entirely — transcript preserved in full — but is
/// intercepted the same way while running.
///
/// act→plan while running is a **pure state switch**: optimistic
/// `fold_agent_switch` + `UiCmd::SwitchAgent` enqueue. The worker is
/// single-threaded and consumes the command only at the next turn boundary,
/// so the in-flight turn finishes under the old agent (`sess.agent` never
/// flips mid-`run_session`), and the worker resets the plan phase when it
/// applies the switch.
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
    if plan_to_act && (*running || chat.subagents_running > 0) {
        // plan→act carries handoff semantics (transcript fold + immediate
        // execution). A turn in flight OR a live subagent makes that unsafe:
        // the worker is mid-run_session and a handoff would start the next
        // turn with a stale agent at an arbitrary partial boundary.
        // Intercept with an explicit hint — the user re-presses at a clean
        // idle boundary (no deferred auto-fire). sys_tokens / input /
        // running are untouched (the agent stays in plan mode).
        *mode_flash = Some((
            "\u{23f3} busy \u{2014} plan switch blocked, retry when idle".into(),
            anim_tick,
        ));
        return SwitchOutcome::Proceed;
    }
    *sys_tokens = sys_tokens_for(&name, workdir, active_skill_body.as_deref());
    // Optimistically reflect the switch so the status chip is correct even if
    // AgentSwitch is dropped under channel pressure. Covers non-turning switches
    // (Alt+Tab) that emit no TurnDone to reconcile against. Folding the full
    // switch (not just copying the name) also collapses a stale
    // `plan_submitted` synchronously — the second tap of a rapid Shift+Tab
    // act→plan→act double-tap then takes the pure-switch branch below instead
    // of firing a bogus handoff off an `AgentSwitch("plan")` event that has
    // not yet round-tripped back to the UI.
    chat.fold_agent_switch(&name);
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
    admit: &mut crate::queue_admitter::AdmitUiState,
    running: &mut bool,
    cancelled: &mut bool,
    drain_pending: &mut bool,
    skip_next_render: &mut bool,
    follow: &mut bool,
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    evt_rx: &mut mpsc::Receiver<UiEvent>,
    _notepad: &mut Option<crate::notepad::NotepadView>,
    question_menu: &mut Option<crate::question_menu::QuestionMenu>,
    question_hub: &std::sync::Arc<opencoder_session::QuestionHub>,
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
                // Question dialogs ride on ToolStart/ToolEnd (no new event
                // kind): open on `question` start, close on its end. Only
                // live events reach here — store replay never opens dialogs.
                match &sev {
                    SessionEvent::ToolStart { id, name, input } if name == "question" => {
                        crate::question_menu::on_tool_start(question_menu, id, input);
                    }
                    SessionEvent::ToolEnd { id, .. } => {
                        crate::question_menu::on_tool_end(question_menu, id, question_hub);
                    }
                    _ => {}
                }
                if let SessionEvent::TranscriptReset(msgs) = &sev {
                    let agent = chat.agent.clone();
                    let saved_plan_submitted = chat.plan_submitted;
                    let saved_annotation_text = chat.annotation_text.clone();
                    let saved_submitted = chat.submitted;
                    let saved_first_prompt = chat.first_prompt.clone();
                    *chat =
                        crate::session_ui::replay_into_chat(&agent, msgs, store, session_id).await;
                    chat.plan_submitted = saved_plan_submitted;
                    chat.annotation_text = saved_annotation_text;
                    chat.submitted = saved_submitted;
                    chat.first_prompt = saved_first_prompt;
                    // The reset happened inside the admitted turn; reliable
                    // completion repair must never target pre-reset blocks.
                    chat.turn_block_start = chat.blocks.len();
                } else {
                    hidden_reasoning_append = matches!(sev, SessionEvent::ReasoningDelta(_))
                        && chat.last_open_thinking_collapsed();
                    chat.apply(&sev);
                }
                if let SessionEvent::QueueConsumed { seq, text } = &sev {
                    // Ledger for optimistic-admit reconciliation: if the drain
                    // consumed a row whose admit completion is still in flight,
                    // the completion must drop (never resurrect) the temp row.
                    crate::queue_admitter::note_consumed(admit, *seq);
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
                        chat.blocks.push(crate::chat::ChatBlock::User {
                            rendered: crate::markdown::render(&display),
                        });
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
                            // Error: re-sync both mirrors from the store (the
                            // same authoritative rebuild Done does) so pending
                            // rows stay visible; they are consumed on the next
                            // submit's drain or a `>` panel drain. Unlike Done
                            // we do NOT arm drain_pending and keep running
                            // false — auto-restarting after an error would
                            // risk error loops.
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
                            *running = false;
                        }
                    }
                }
            }
            UiEvent::AssistantFinal(text) => {
                chat.reconcile_completed_assistant(&text);
            }
            UiEvent::TurnDone(agent) => {
                // Reconcile the status chip from the authoritative agent.
                // The ordered forwarder reliably delivers AgentSwitch before
                // TurnDone. Keep this authoritative assignment for compatibility
                // with older producers and restored UI state.
                //
                // Consumption-time plan arm: a TurnDone(plan) means a turn
                // actually RAN in the plan phase, so re-arm `plan_submitted`
                // from the persisted plan-phase counter — the authoritative
                // record of delivered requirements (incremented when a real
                // requirement is recorded for the plan agent; bare commands
                // and skill-only submissions never increment it). This covers
                // steers, queued inputs and compound `/plan <content>` (the
                // counter persists at record time, before this TurnDone),
                // and it can NEVER arm from a stranded, never-consumed
                // admit. A store failure keeps the current flag (fail-open).
                if agent == "plan" {
                    if let Ok(Some(meta)) = store.get_session(session_id).await {
                        chat.plan_submitted =
                            meta.agent.as_deref() == Some("plan") && meta.plan_input_count > 0;
                    }
                }
                chat.agent = crate::terminal_text::sanitize_single_line(&agent).into_owned();
                // Safety net for older producers that could omit
                // SessionEvent::Done during token bursts. Current TurnDone is
                // reliably delivered by the ordered forwarder, and
                // finalize_assistant is idempotent
                // (the `!*done` guard), so re-calling when Done was
                // already processed is a no-op.
                chat.finalize_assistant();
                // Reconcile orphaned subagents from cancellation, restored old
                // state, or older lossy producers. TurnDone is the authoritative
                // idle boundary for this compatibility repair.
                chat.reconcile_orphaned_subagents();
                if *drain_pending {
                    // The cancelled turn has finished draining — restart
                    // the drain loop to promote pending steers.
                    *drain_pending = false;
                    *cancelled = false;
                    if !start_turn(cmd_tx, cancel, UiCmd::Prompt(String::new(), Vec::new())).await {
                        worker_dead(chat);
                        return LoopFlow::Quit;
                    }
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
    mcp_menu: &mut Option<crate::mcp_menu::McpMenu>,
    envs_menu: &mut Option<crate::envs_menu::EnvsMenu>,
    cli_menu: &mut Option<crate::cli_menu::CliMenu>,
    skill_toggle_menu: &mut Option<crate::skill_menu::SkillMenu>,
    ap_menu: &mut Option<crate::ap_menu::ApMenu>,
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
    notepad: &mut Option<crate::notepad::NotepadView>,
) -> LoopFlow {
    let (outcome, quit) = handle_command_key(command_menu, k);
    if quit {
        let _ = cmd_tx.send(UiCmd::Quit).await;
        return LoopFlow::Quit;
    }
    match outcome {
        CommandOutcome::Dispatch(action) => {
            return dispatch_slash_action(
                action,
                cmd_tx,
                cancel,
                chat,
                running,
                follow,
                store,
                session_id,
                task_picker,
                model_menu,
                mcp_menu,
                envs_menu,
                cli_menu,
                skill_toggle_menu,
                ap_menu,
                cache_salt_menu,
                agent_name,
                input,
                cursor_idx,
                config,
                workdir,
                mode_flash,
                anim_tick,
                sys_tokens,
                plan_edit,
                notepad,
            )
            .await;
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

/// Handle a key in plan/annotation-edit mode. On Exit, persists iff modified.
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
                    let _ = cmd_tx.send(crate::worker::UiCmd::EditPlan(text)).await;
                }
                crate::plan_edit::EditKind::Annotation => {
                    chat.update_annotation_text(&text);
                    let _ = cmd_tx
                        .send(crate::worker::UiCmd::EditAnnotation(text))
                        .await;
                }
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

#[cfg(test)]
pub(crate) use app_loop_model::env_model_override;
pub(crate) use app_loop_model::handle_model_outcome;

#[path = "app_loop_mcp.rs"]
mod app_loop_mcp;

pub(crate) use app_loop_mcp::handle_mcp_outcome;

#[path = "app_loop_envs.rs"]
mod app_loop_envs;

pub(crate) use app_loop_envs::handle_envs_outcome;

#[path = "app_loop_cli.rs"]
mod app_loop_cli;

pub(crate) use app_loop_cli::handle_cli_outcome;

#[path = "app_loop_skill.rs"]
mod app_loop_skill;

pub(crate) use app_loop_skill::handle_skill_outcome;

#[path = "app_loop_ap.rs"]
mod app_loop_ap;

pub(crate) use app_loop_ap::handle_ap_outcome;

#[path = "app_loop_paste.rs"]
mod app_loop_paste;

#[cfg(test)]
pub(crate) use app_loop_paste::route_paste;
pub(crate) use app_loop_paste::{handle_paste_event, paste_clipboard_image};

#[path = "app_loop_actions.rs"]
mod app_loop_actions;

pub(crate) use app_loop_actions::{
    cancel_running_turn, dispatch_slash_action, steer_submit_after_mouse,
};

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
    use crate::keymap_menu::handle_keymap_key;
    let outcome = handle_keymap_key(keymap_menu, k);
    apply_keymap_outcome(outcome, config, keymap, workdir, cmd_tx).await
}

/// Apply a [`KeymapOutcome`] (from keyboard or mouse) with shared side-effects.
pub(crate) async fn apply_keymap_outcome(
    outcome: crate::keymap_menu::KeymapOutcome,
    config: &mut Config,
    keymap: &mut crate::keymap::KeyBindings,
    workdir: &Path,
    cmd_tx: &mpsc::Sender<UiCmd>,
) -> LoopFlow {
    use crate::keymap_menu::KeymapOutcome;
    match outcome {
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

/// Handle a mouse event while the keymap modal is open.
pub(crate) async fn handle_keymap_mouse_event(
    keymap_menu: &mut Option<KeymapMenu>,
    btn_rects: &[ratatui::layout::Rect],
    m: &crossterm::event::MouseEvent,
    config: &mut Config,
    keymap: &mut crate::keymap::KeyBindings,
    workdir: &Path,
    cmd_tx: &mpsc::Sender<UiCmd>,
) -> LoopFlow {
    let outcome = crate::keymap_menu::mouse::handle_keymap_mouse(
        keymap_menu,
        btn_rects,
        m.column,
        m.row,
        &m.kind,
    );
    apply_keymap_outcome(outcome, config, keymap, workdir, cmd_tx).await
}
