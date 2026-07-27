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
use std::time::{Duration, Instant};

use crossterm::event::KeyEvent;
use opencoder_core::Config;
use opencoder_llm::ChatStream;
use opencoder_session::SessionEvent;
use opencoder_store::Store;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app_helpers::{paste_payload, start_turn, sys_tokens_for, worker_dead};
use crate::cache_salt_menu::CacheSaltMenu;
use crate::chat::ChatView;
use crate::command::{handle_command_key, CommandMenu, CommandOutcome, SlashAction};
use crate::composer;
use crate::model_menu::{handle_model_key, ConfigForm, ModelMenu, ModelOutcome, ProviderList};
use crate::model_session_switch::switch_session;
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
    *sys_tokens = sys_tokens_for(&name, workdir, active_skill_body.as_deref());
    if plan_to_act && chat.plan_submitted {
        if *running {
            // Plan turn still running — Shift+Tab is a no-op.
            *mode_flash = Some(("\u{21bb} plan running\u{2026}".into(), anim_tick));
            return SwitchOutcome::Proceed;
        }
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
                    // Mirror the SteerConsumed marker: resolve seq->prompt,
                    // embed a `queued: {prompt}` marker so the user sees WHEN
                    // the queued follow-up fired, then drop the row.
                    if let Some(prompt) = queue_items
                        .iter()
                        .find(|(s, _)| s == seq)
                        .map(|(_, p)| p.clone())
                    {
                        chat.push_marker(Line::from(Span::styled(
                            format!("queued: {prompt}"),
                            Style::default()
                                .fg(Color::Yellow)
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
                        *running = false;
                        chat.steer_items.clear();
                        // Only clear queue_items on Done — the store queue is provably
                        // empty (claim_one_queued returned None before Done). On Error,
                        // queued items may still be pending in the store; they are
                        // maintained per-item by QueueConsumed events and will be
                        // consumed on the next drain.
                        if matches!(sev, SessionEvent::Done) {
                            queue_items.clear();
                        }
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
                            *model_label = reloaded.model.clone();
                            *context_limit = reloaded.context_limit();
                            // Rebuild the outer `client` too so subsequent
                            // `/task` new sessions pick up the new endpoint
                            // (the worker only swaps its own sess.client).
                            match reloaded.resolve_endpoint() {
                                Ok(ep) => match opencoder_llm::ChatClient::new(
                                    &ep.base_url,
                                    &ep.api_key,
                                    &ep.headers,
                                    reloaded.network.proxy.as_deref(),
                                ) {
                                    Ok(new_client) => {
                                        *client = Arc::new(new_client);
                                    }
                                    Err(e) => {
                                        chat.push_marker(Line::from(Span::styled(
                                            format!(
                                                "[/config] client build failed: {e:#} — \
                                                 live session keeps previous client"
                                            ),
                                            Style::default().fg(Color::Red),
                                        )));
                                    }
                                },
                                Err(e) => {
                                    chat.push_marker(Line::from(Span::styled(
                                        format!(
                                            "[/config] endpoint resolve failed: {e:#} — \
                                             live session keeps previous client"
                                        ),
                                        Style::default().fg(Color::Red),
                                    )));
                                }
                            }
                            *config = reloaded.clone();
                            let effective_model = reloaded.model.clone();
                            // Apply a new TUI frame rate immediately: rebuild the frame
                            // interval so the just-saved fps takes effect without restart.
                            let new_frame_ms = reloaded.tui_frame_ms();
                            if new_frame_ms != *frame_ms {
                                *frame_ms = new_frame_ms;
                                *frame_ticker =
                                    tokio::time::interval(Duration::from_millis(*frame_ms));
                                frame_ticker.set_missed_tick_behavior(
                                    tokio::time::MissedTickBehavior::Skip,
                                );
                            }
                            let _ = cmd_tx.send(UiCmd::ReloadConfig(Box::new(reloaded))).await;
                            chat.push_marker(Line::from(Span::styled(
                                format!("[/config] saved \u{2192} {}", path.display()),
                                Style::default().fg(Color::Green),
                            )));
                            // Issue #2: if an exported OPENCODER_MODEL silently
                            // reverted this /model switch, surface it instead of
                            // leaving the status bar on an unexpected model.
                            if let Some(env_val) = env_model_override(
                                json.get("model").and_then(|v| v.as_str()),
                                &effective_model,
                                std::env::var("OPENCODER_MODEL").ok().as_deref(),
                            ) {
                                chat.push_marker(Line::from(Span::styled(
                                    format!(
                                        "[config] OPENCODER_MODEL is set ({env_val}) \u{2014} \
                                         /model switch overridden by env"
                                    ),
                                    Style::default().fg(Color::Red),
                                )));
                            }
                        }
                        Err(e) => {
                            chat.push_marker(Line::from(Span::styled(
                                format!("[/config] reload failed: {e:#}"),
                                Style::default().fg(Color::Red),
                            )));
                        }
                    }
                }
                Err(e) => {
                    chat.push_marker(Line::from(Span::styled(
                        format!("[/config] save failed: {e:#}"),
                        Style::default().fg(Color::Red),
                    )));
                }
            }
        }
        ModelOutcome::SaveSessionOnly(json) => {
            switch_session(json, config, client, cmd_tx, chat).await;
            *model_label = config.model.clone();
            *context_limit = config.context_limit();
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
                    Style::default().fg(Color::Yellow),
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
        CommandOutcome::Idle => {}
    }
    LoopFlow::Proceed
}

/// Paste an image (screenshot bitmap) from the system clipboard into the
/// composer's `pending_images`. Triggered by `Ctrl+V`. Replaces the former
/// `/clip` slash command. The blocking `arboard` clipboard read runs on a
/// background thread so it can't stall the async event loop.
pub(crate) async fn paste_clipboard_image(
    chat: &mut ChatView,
    pending_images: &mut Vec<(String, String)>,
) -> LoopFlow {
    match tokio::task::spawn_blocking(crate::clipboard::clipboard_image_data_uri).await {
        Ok(Some(data_uri)) => {
            let n = pending_images.len() + 1;
            pending_images.push((data_uri, "clipboard.png".to_string()));
            chat.push_marker(Line::from(Span::styled(
                format!("\u{1f4ce} pasted image from clipboard ({n} attached)"),
                Style::default().fg(Color::Green),
            )));
        }
        Ok(None) => {
            chat.push_marker(Line::from(Span::styled(
                "[clip] no image in clipboard",
                Style::default().fg(Color::Yellow),
            )));
        }
        Err(e) => {
            chat.push_marker(Line::from(Span::styled(
                format!("[clip] clipboard read failed: {e}"),
                Style::default().fg(Color::Red),
            )));
        }
    }
    LoopFlow::Proceed
}

/// Route a paste payload by the same modal priority as key events: an open
/// popup owns the paste, so it never reaches the main input hidden behind it.
///
/// Mirrors [`Event::Key`](crossterm::event::Event::Key)'s priority chain:
/// - task picker / cache-salt menu open -> modal isolation (no text fields),
///   swallow the paste;
/// - model menu open -> feed the trimmed payload to its focused field via
///   [`ModelMenu::paste`];
/// - command menu open -> append to its query and refilter via
///   [`CommandMenu::paste`];
/// - otherwise -> resolve a dragged file to its absolute path (or insert the
///   payload verbatim) into the main composer; image file paths are loaded
///   into `pending_images` as data URIs instead.
///
/// Returns [`LoopFlow::Redraw`] when a modal consumed the paste (the caller
/// re-renders), [`LoopFlow::Proceed`] when the main composer absorbed it
/// (`input`/`cursor_idx` updated in place). Never returns `Quit`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn route_paste(
    pasted: &str,
    task_picker_open: bool,
    cache_salt_menu_open: bool,
    model_menu: &mut Option<ModelMenu>,
    command_menu: &mut Option<CommandMenu>,
    input: &mut String,
    cursor_idx: &mut usize,
    pending_images: &mut Vec<(String, String)>,
    workdir: &Path,
) -> LoopFlow {
    if task_picker_open || cache_salt_menu_open {
        // No text fields here -- modal isolation: swallow the paste.
        return LoopFlow::Redraw;
    }
    // Modal fields never resolve drag-and-drop file paths; only strip the
    // trailing newline terminals append to pasted payloads.
    let trimmed = pasted.trim_end_matches(['\r', '\n']);
    if let Some(menu) = model_menu.as_mut() {
        menu.paste(trimmed);
        return LoopFlow::Redraw;
    }
    if let Some(menu) = command_menu.as_mut() {
        menu.paste(trimmed);
        return LoopFlow::Redraw;
    }
    // Main composer: check if pasted content is an image file path.
    if let Some((data_uri, filename)) = crate::image_util::try_load_image(trimmed, workdir) {
        pending_images.push((data_uri, filename));
        return LoopFlow::Proceed;
    }
    // Otherwise: drag a file in (or clipboard paste) arrives as one
    // atomic payload; resolve an existing file to its absolute path, else
    // insert verbatim.
    let payload = paste_payload(pasted, workdir);
    let (new_input, new_idx) = composer::insert_str(input, *cursor_idx, &payload);
    *input = new_input;
    *cursor_idx = new_idx;
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
            Style::default().fg(Color::Yellow),
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
        *mode_flash = Some(("edit plan \u{2014} :wq save, :q! discard".into(), anim_tick));
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
