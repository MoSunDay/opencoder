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

use crate::app_helpers::{start_turn, worker_dead};
use crate::cache_salt_menu::CacheSaltMenu;
use crate::chat::ChatView;
use crate::command::{handle_command_key, CommandMenu, CommandOutcome};
use crate::keymap_menu::KeymapMenu;
use crate::model_menu::ModelMenu;
use crate::task::TaskPicker;
use crate::theme;
use crate::worker::{UiCmd, UiEvent};

/// Animation tick rate for the running spinner (10 FPS).
pub(crate) const ANIM_TICK_MS: u64 = 100;
/// Body refresh cadence (3 FPS), decoupled from the fast spinner.
pub(crate) const BODY_REFRESH_MS: u64 = 333;

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

/// Display-state computation lives in [`crate::app::app_display`] beside the
/// title composer it relies on (extracted to keep this file within the
/// iteration line cap). Re-exported so `app.rs` / tests keep the
/// `app_loop::compute_display` path.
pub(crate) use crate::app::app_display::{compute_display, DisplayState};
// Test-only path stability: in-file test modules keep `super::body_is_top_level`.
#[cfg(test)]
pub(crate) use crate::app::app_display::body_is_top_level;


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

/// Whether an input event should prompt an immediate frame render.
///
/// The main loop renders only when `dirty && render_pending`; before this
/// predicate was wired in, the only steady-state source of `render_pending`
/// was the fps-configured `frame_ticker` (default 10 FPS = 100ms/帧), so key
/// echo waited an average ~50ms (worst ~100ms) — visibly laggy for IME/CJK
/// input where the user watches the screen to confirm each commit. Marking
/// every Key/Paste/Mouse/Resize event as render-prompting decouples input
/// echo latency from the fps setting (frames are ratatui diffs, so unchanged
/// state renders an empty diff; idle CPU is unaffected).
pub(crate) fn input_event_prompts_frame(ev: &crossterm::event::Event) -> bool {
    matches!(
        ev,
        crossterm::event::Event::Key(_)
            | crossterm::event::Event::Mouse(_)
            | crossterm::event::Event::Paste(_)
            | crossterm::event::Event::Resize(_, _)
    )
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
    plan_skill_active: &mut bool,
    admit: &mut crate::queue_admitter::AdmitUiState,
    running: &mut bool,
    cancelled: &mut bool,
    drain_pending: &mut bool,
    skip_next_render: &mut bool,
    follow: &mut bool,
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    turn_gen: &mut u64,
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
            UiEvent::RemoteSnapshot {
                chat: restored,
                running: active,
            } => {
                *chat = *restored;
                *running = active;
                *cancelled = false;
                *drain_pending = false;
                queue_items.clear();
            }
            UiEvent::Session(sev) => {
                // Question dialogs ride on ToolStart/ToolEnd (no new event
                // kind): open on `question` start, close on its end. Only
                // live events reach here — store replay never opens dialogs.
                match &sev {
                    SessionEvent::ToolStart { id, name, input } if name == "question" => {
                        crate::question_menu::on_tool_start(question_menu, id, input);
                        crate::hooks::emit(crate::hooks::Event::Question);
                    }
                    SessionEvent::ToolEnd { id, .. } => {
                        crate::question_menu::on_tool_end(question_menu, id, question_hub);
                    }
                    _ => {}
                }
                if chat.remote
                    && matches!(
                        sev,
                        SessionEvent::LlmRoundStart { .. } | SessionEvent::TextDelta(_)
                    )
                {
                    *running = true;
                }
                if let SessionEvent::TranscriptReset(msgs) = &sev {
                    if chat.remote {
                        let label = chat.agent.clone();
                        *chat = crate::session_ui::replay_messages(&label, msgs);
                        chat.remote = true;
                        chat.submitted = true;
                    } else {
                        crate::session_ui::rebuild_after_reset(chat, msgs, store, session_id).await;
                    }
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
                    // The event text is already model-facing: the compound
                    // tail for `/plan <args>`, empty for a bare control
                    // command (applied inline, never echoed). Normalize again
                    // here so legacy persisted events carrying the raw
                    // prefix stay correct too; the local mirror falls back
                    // through the same normalization.
                    let display = if !text.is_empty() {
                        opencoder_session::consumed_echo_text(text)
                    } else {
                        queue_items
                            .iter()
                            .find(|(s, _)| s == seq)
                            .and_then(|(_, d)| opencoder_session::consumed_echo_text(d))
                    };
                    if let Some(display) = display {
                        if !display.is_empty() {
                            chat.blocks.push(crate::chat::ChatBlock::User {
                                rendered: crate::markdown::render(&display),
                            });
                            chat.push_marker(Line::from(""));
                            // Remember the echo across a TranscriptReset
                            // rebuild (a queued `/act_clear_context <tail>`
                            // resets the view right after this event).
                            chat.pending_turn_echo = Some(display.clone());
                            // The queued prompt opens a NEW Turn: re-anchor
                            // the ladder floor BELOW the echo (`begin_turn`
                            // ran at the drain restart, before the echo
                            // landed) so this turn's steps render as their
                            // own `N Steps` group after the prompt — never
                            // above it, never merged with the previous
                            // turn's group.
                            chat.reanchor_turn_after_user_echo();
                        }
                    }
                    queue_items.retain(|(s, _)| s != seq);
                    // A queued input actually took effect: re-derive the
                    // task-plan highlight from the consumed text -- a
                    // `$task-plan` token in it is newly activated by the
                    // runner's record_compound and keeps the chip yellow; any
                    // other consumed input reverts the chip to the plain hue.
                    *plan_skill_active =
                        crate::skill_persist::plan_highlight_from_consumed_text(text);
                }
                if let SessionEvent::SteerConsumed { seq, text } = &sev {
                    // Ledger for optimistic-admit reconciliation: if the drain
                    // consumed a steer whose admit completion is still in
                    // flight, the completion must drop (never resurrect) the
                    // temp row.
                    crate::queue_admitter::note_consumed(admit, *seq);
                    // A steered input actually took effect: same re-derivation
                    // as the queue path -- a `$task-plan` token lights the
                    // chip, any other steered input reverts it.
                    *plan_skill_active =
                        crate::skill_persist::plan_highlight_from_consumed_text(text);
                }
                if chat.remote && matches!(sev, SessionEvent::Done | SessionEvent::Error(_)) {
                    *running = false;
                    *cancelled = false;
                    *drain_pending = false;
                } else if matches!(sev, SessionEvent::Done | SessionEvent::Error(_)) {
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
            UiEvent::TurnDone(agent, turn) => {
                // Stale completion guard: the worker emits exactly one
                // TurnDone per processed turn command, tagged with the
                // generation of its most recent ResetCancel. When the app
                // loop has since cancelled (cancel_running_turn bumps the
                // generation) or started a newer turn, this completion
                // belongs to the OLD turn — dropping it keeps `running`
                // armed for the superseding turn (double-Shift+Tab act fire)
                // and keeps the chip from snapping back mid-handoff.
                if *turn_gen != turn {
                    continue;
                }
                // Reconcile the status chip from the authoritative agent.
                // The ordered forwarder reliably delivers AgentSwitch before
                // TurnDone. Keep this authoritative assignment for compatibility
                // with older producers and restored UI state.
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
                    if !start_turn(cmd_tx, cancel, turn_gen, UiCmd::Prompt(String::new(), Vec::new()))
                        .await
                    {
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
                    crate::hooks::emit(crate::hooks::Event::TurnDone);
                }
            }
            // Subtask watcher deliveries never arrive here: they travel on
            // the dedicated `SubtaskUi` channel drained by the app loop
            // (workers are quit on every session switch, so the session
            // worker channel cannot carry them across switches).
            UiEvent::SubtaskUpdate { .. } => {}
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
    turn_gen: &mut u64,
    chat: &mut ChatView,
    sidecar_ask: &mpsc::Sender<crate::sidecar_ui::SidecarCmd>,
    running: &mut bool,
    follow: &mut bool,
    store: &Arc<dyn Store>,
    session_id: &str,
    task_picker: &mut Option<TaskPicker>,
    model_menu: &mut Option<ModelMenu>,
    mcp_menu: &mut Option<crate::mcp_menu::McpMenu>,
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
    clear_confirm: &mut Option<crate::clear_confirm::ClearConfirm>,
    agent_menu: &mut Option<crate::agent_menu::AgentMenu>,
    subtask_view: &mut Option<crate::subtask_detail::SubtaskView>,
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
                sidecar_ask,
                running,
                follow,
                store,
                session_id,
                task_picker,
                model_menu,
                mcp_menu,
                cli_menu,
                skill_toggle_menu,
                ap_menu,
                cache_salt_menu,
                agent_name,
                config,
                workdir,
                mode_flash,
                anim_tick,
                sys_tokens,
                plan_edit,
                notepad,
                clear_confirm,
                agent_menu,
                subtask_view,
                turn_gen,
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
/// to the 30-min timeout), freezing the terminal on the alt-screen. `busy`
/// covers BOTH a running parent turn and the subagent drain window (parent
/// already idle, children still live) — in that gap the worker can still be
/// unwinding inside `run_session`, so quitting without the token cancel
/// wedges the same way.
pub(crate) async fn handle_quit(
    busy: bool,
    cancel: &CancellationToken,
    chat: &mut ChatView,
    cmd_tx: &mpsc::Sender<UiCmd>,
) {
    if busy && !chat.remote {
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
        *mode_flash = Some(("\u{2192} edit plan".into(), anim_tick));
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

#[cfg(test)]
#[path = "app_loop_render_prompt_tests.rs"]
mod render_prompt_tests;

#[path = "app_loop_model.rs"]
mod app_loop_model;

#[cfg(test)]
pub(crate) use app_loop_model::env_model_override;
pub(crate) use app_loop_model::handle_model_outcome;

#[path = "app_loop_mcp.rs"]
mod app_loop_mcp;

pub(crate) use app_loop_mcp::handle_mcp_outcome;

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

#[cfg(test)]
pub(crate) use app_loop_actions::fire_clear_confirm;
pub(crate) use app_loop_actions::{
    append_subtask_result, cancel_running_turn, confirm_tick, dispatch_mode_switch,
    dispatch_slash_action, handle_confirm_key, steer_submit_after_mouse, ModeSwitch,
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
