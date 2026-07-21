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
use std::time::Duration;

use crossterm::event::KeyEvent;
use opencoder_core::Config;
use opencoder_llm::ChatStream;
use opencoder_session::SessionEvent;
use opencoder_store::Store;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app_helpers::{start_turn, worker_dead};
use crate::cache_salt_menu::CacheSaltMenu;
use crate::chat::ChatView;
use crate::command::{handle_command_key, CommandMenu, CommandOutcome, SlashAction};
use crate::model_menu::{handle_model_key, ConfigForm, ModelMenu, ModelOutcome, ProviderList};
use crate::task::TaskPicker;
use crate::worker::{gate_compact, CompactGate, UiCmd, UiEvent};

/// Translation of the `continue` / `break` control flow that lived inside the
/// extracted loop blocks. `Proceed` means fall through to the rest of the loop
/// body (the block did neither `continue` nor `break`); `Redraw` was a
/// `continue` (jump to the next turn, re-render); `Quit` was a `break`
/// (exit the loop).
pub(crate) enum LoopFlow {
    Proceed,
    /// Used by extracted blocks that previously did `continue` (re-render).
    #[allow(dead_code)] // constructed by a later-extracted block
    Redraw,
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
    model_label: &str,
) -> DisplayState<'a> {
    let agent_name = chat.agent.clone();
    let status = chat.status.clone();
    // When viewing a subagent's perspective, swap in its child ChatView,
    // back-title, and its own context stats (instead of the parent's).
    // The body title keeps the "Esc back" hint; the status bar uses the
    // short subagent kind so it renders the same layout as the parent.
    let (display_chat, display_title, display_status_agent, display_ctx, display_sys) =
        if let Some(idx) = subagent_focus {
            match chat.blocks.get(idx) {
                Some(crate::chat::ChatBlock::Subagent {
                    view, kind, prompt, ..
                }) => (
                    view as &crate::chat::ChatView,
                    format!("\u{2190} [Esc] back | \u{2937}sub [{kind}] {prompt}"),
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
                agent_name.clone(),
                agent_name.clone(),
                chat.context_used,
                sys_tokens,
            )
        };
    // Compose status-bar model label with reasoning-effort badge (e.g.
    // "glm-5.2 \u{00b7}high") so the active thinking depth is visible.
    let status_model = match &config.reasoning_effort {
        Some(e) if !e.trim().is_empty() => format!("{model_label} \u{00b7}{e}"),
        _ => model_label.to_string(),
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
                    *chat =
                        crate::session_ui::replay_into_chat(&agent, msgs, store, session_id).await;
                } else {
                    chat.apply(&sev);
                    if matches!(sev, SessionEvent::ReasoningDelta(_))
                        && chat.last_thinking_collapsed()
                    {
                        *skip_next_render = true;
                    }
                }
                if let SessionEvent::QueueConsumed { seq } = &sev {
                    queue_items.retain(|(s, _)| s != seq);
                }
                if matches!(sev, SessionEvent::Done | SessionEvent::Error(_)) {
                    if *cancelled {
                        // Stale event from a cancelled turn — consume without
                        // affecting running or clearing items belonging to a
                        // potentially-new turn.
                        *cancelled = false;
                    } else if !*drain_pending {
                        *running = false;
                        chat.steer_items.clear();
                        queue_items.clear();
                    }
                }
            }
            UiEvent::TurnDone => {
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
                    start_turn(cmd_tx, cancel, UiCmd::Prompt(String::new())).await;
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
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_model_outcome(
    model_menu: &mut Option<ModelMenu>,
    k: KeyEvent,
    client: &mut Arc<dyn ChatStream>,
    config: &mut Config,
    model_label: &mut String,
    context_limit: &mut u64,
    frame_ms: &mut u64,
    frame_ticker: &mut tokio::time::Interval,
    cmd_tx: &mpsc::Sender<UiCmd>,
    chat: &mut ChatView,
    workdir: &Path,
) -> LoopFlow {
    match handle_model_key(model_menu, k) {
        ModelOutcome::Save(json) => {
            match Config::save(workdir, &json) {
                Ok(path) => {
                    match Config::load(workdir) {
                        Ok(reloaded) => {
                            *model_label = reloaded.model_id().to_string();
                            *context_limit = reloaded.context_limit();
                            // Rebuild the outer `client` too so subsequent
                            // `/task` new sessions pick up the new endpoint
                            // (the worker only swaps its own sess.client).
                            if let Ok(ep) = reloaded.resolve_endpoint() {
                                if let Ok(new_client) = opencoder_llm::ChatClient::new(
                                    &ep.base_url,
                                    &ep.api_key,
                                    &ep.headers,
                                    reloaded.network.proxy.as_deref(),
                                ) {
                                    *client = Arc::new(new_client);
                                }
                            }
                            *config = reloaded.clone();
                            // Apply a new TUI frame rate immediately: rebuild the frame
                            // interval so the just-saved fps takes effect without restart.
                            let new_frame_ms = reloaded.tui_frame_ms();
                            if new_frame_ms != *frame_ms {
                                *frame_ms = new_frame_ms;
                                *frame_ticker = tokio::time::interval(Duration::from_millis(*frame_ms));
                                frame_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                            }
                            let _ = cmd_tx.send(UiCmd::ReloadConfig(Box::new(reloaded))).await;
                            chat.push_marker(Line::from(Span::styled(
                                format!("[/config] saved \u{2192} {}", path.display()),
                                Style::default().fg(Color::Green))));
                        }
                        Err(e) => {
                            chat.push_marker(Line::from(Span::styled(
                                format!("[/config] reload failed: {e:#}"),
                                Style::default().fg(Color::Red))));
                        }
                    }
                }
                Err(e) => {
                    chat.push_marker(Line::from(Span::styled(
                        format!("[/config] save failed: {e:#}"),
                        Style::default().fg(Color::Red))));
                }
            }
        }
        ModelOutcome::Cancel | ModelOutcome::Idle => {}
        ModelOutcome::Quit => {
            let _ = cmd_tx.send(UiCmd::Quit).await;
            return LoopFlow::Quit;
        }
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
    config: &Config,
    cache_salt_menu: &mut Option<CacheSaltMenu>,
    agent_name: &str,
) -> LoopFlow {
    let (outcome, quit) = handle_command_key(command_menu, k);
    if quit {
        let _ = cmd_tx.send(UiCmd::Quit).await;
        return LoopFlow::Quit;
    }
    match outcome {
        CommandOutcome::Dispatch(SlashAction::Task) => {
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
        CommandOutcome::Dispatch(SlashAction::Compact) => {
            match gate_compact(*running) {
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
                        Style::default().fg(Color::Yellow),
                    )));
                }
            }
        }
        CommandOutcome::Dispatch(SlashAction::CacheSalt) => {
            let enabled = config.cache_salt == Some(true);
            *cache_salt_menu = Some(
                match CacheSaltMenu::build(store.as_ref(), session_id, agent_name, enabled)
                    .await
                {
                    Ok(m) => m,
                    Err(_) => CacheSaltMenu::parent_only(agent_name, session_id, enabled),
                },
            );
        }
        CommandOutcome::Idle => {}
    }
    LoopFlow::Proceed
}
