use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::Event;
use opencoder_core::Config;
use opencoder_llm::{estimate, ChatStream};
use opencoder_session::SessionState;
use opencoder_store::{Delivery, Store};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cache_salt_menu::{handle_cache_salt_key, CacheSaltMenu, CacheSaltOutcome};
use crate::chat::ChatView;
use crate::command::CommandMenu;
use crate::input::spawn_input_pump;
use crate::key_handler::{handle_key, KeyAction};
use crate::menu::SkillMenu;
use crate::model_menu::ModelMenu;
use crate::render::{MouseHits, Term};
use crate::task::{handle_task_key, TaskOutcome, TaskPicker};
use crate::worker::{process_cmd, UiCmd, UiEvent};
use crate::TuiOpts;

#[path = "app_loop.rs"]
pub(crate) mod app_loop;

#[path = "app_task.rs"]
mod app_task;

#[path = "app_bootstrap.rs"]
mod app_bootstrap;

#[path = "subagent_input.rs"]
mod subagent_input;

/// Animation tick rate for the running spinner (10 FPS).
const ANIM_TICK_MS: u64 = 100;
/// Body (info area) refresh interval -- the cached ChatView snapshot is rebuilt
/// at this cadence (3 FPS), decoupling text layout from the fast spinner.
const BODY_REFRESH_MS: u64 = 333;

pub async fn run(opts: &TuiOpts) -> Result<()> {
    app_bootstrap::run(opts).await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_app(
    terminal: &mut Term,
    session: SessionState,
    store: Arc<dyn Store>,
    mut session_id: String,
    mut context_limit: u64,
    mut model_label: String,
    workdir: PathBuf,
    mut config: Config,
    mut client: Arc<dyn ChatStream>,
) -> Result<String> {
    // Wire a cancellation token into the session so double-Esc can hard-abort
    // the running turn (mid-stream / mid-tool). The UI keeps a clone to signal.
    // `mut`: reassigned by `rebind_session` on every `/task` session switch.
    let mut cancel = CancellationToken::new();
    let session = session.with_cancel(cancel.clone());
    let child_turn_cancels = session.child_turn_cancels.clone();
    let mut skill_handle = session.skill_prompt.clone();

    let mut chat = initial_chat_view(&session, &store).await;
    let mut input = String::new();
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut cursor_idx: usize = 0;
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut running = false;
    let mut run_elapsed_ms: u64 = 0;
    let mut last_clock = Instant::now();
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut show_help = false;
    let mut help_scroll: u16 = 0;
    let mut undo_state = crate::undo::init(&input, cursor_idx);
    let mut scroll: u32 = 0;
    let mut follow = true;
    let mut plan_edit: Option<crate::plan_edit::PlanEdit> = None;
    let mut sys_tokens: u64 = sys_tokens_for(session.agent.name.as_str(), &workdir, None);
    // Cached system-prompt tokens for the subagent currently being viewed.
    // Computed once on entry (ctx-switch click) to avoid per-frame rebuild.
    let mut subagent_sys: u64 = 0;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut skill_menu: Option<SkillMenu> = None;
    let mut task_picker: Option<TaskPicker> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut model_menu: Option<ModelMenu> = None;
    let mut cache_salt_menu: Option<CacheSaltMenu> = None;
    let mut active_skill: Option<String> = None;
    let mut active_skill_body: Option<String> = None;
    let mut anim_tick: u32 = 0;
    let mut mode_flash: Option<(String, u32)> = None;
    let mut last_esc: Option<Instant> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut parent_scroll: u32 = 0;
    let mut parent_follow: bool = true;
    // Active mouse text-selection in the body (absolute content-row range), or
    // None. Kept in absolute rows so it tracks the text while the viewport
    // scrolls. Cleared on copy (mouse-up) and on subagent ctx-switch.
    let mut selection: Option<crate::selection::SelRange> = None;
    // Transient copy-feedback message shown for ~2s after a mouse-drag copy,
    // stamped with the instant it was set for timeout-based expiry. Uses
    // `Instant` rather than `anim_tick` because the latter only advances while
    // `running` is true, so a copy during idle would never expire.
    let mut copy_status: Option<(String, Instant)> = None;
    // Double-click detection: timestamp of the last left-click and whether the
    // current selection originated from a double-click (forces copy even for a
    // single-line / lo==hi selection).
    let mut last_click: Option<Instant> = None;
    let mut dbl_click: bool = false;
    // Per-session UI state snapshots — saved on `/task` switch, restored on return.
    let mut session_states: std::collections::HashMap<String, crate::session_ui::SessionUiState> =
        std::collections::HashMap::new();

    let (mut cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(512);

    let worker = tokio::spawn(async move {
        let mut sess = session;
        while let Some(cmd) = cmd_rx.recv().await {
            if process_cmd(cmd, &mut sess, &evt_tx).await {
                break;
            }
        }
    });

    // Terminal input is collected by a dedicated OS thread (bounded
    // `poll`+`read`) and delivered here over `input_rx` — see `crate::input`.
    //
    // Liveness supervisor: crossterm 0.28's mio source busy-loops forever
    // when the pty master closes (SSH drop / pane kill) — it holds the global
    // event mutex, so our `poll(150ms)` never returns and the collector thread
    // stops bumping its heartbeat. The supervisor (a separate OS thread, immune
    // to runtime starvation) detects the stall + termination signals and restores
    // the terminal + exits cleanly instead of leaving a frozen screen.
    let heartbeat = crate::supervisor::Heartbeat::new();
    let supervisor_active = Arc::new(AtomicBool::new(true));
    crate::supervisor::spawn(heartbeat.clone(), Arc::clone(&supervisor_active));
    let (mut input_rx, _input_handle) = spawn_input_pump(heartbeat);
    let mut anim_ticker = tokio::time::interval(Duration::from_millis(ANIM_TICK_MS));
    // Frame-rate limiter: redraw cadence is decided by the `/config` fps
    // (default 10 FPS). `Skip` prevents burst-fire catch-up after a stall.
    let mut frame_ms = config.tui_frame_ms();
    let mut frame_ticker = tokio::time::interval(Duration::from_millis(frame_ms));
    frame_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Body cache refresh ticker: rebuilds the cached ChatView snapshot at 3 FPS.
    // `Skip` prevents burst-fire catch-up after a stall.
    let mut body_ticker = tokio::time::interval(Duration::from_millis(BODY_REFRESH_MS));
    body_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut quitting = false; // render "shutting down…" frame before worker-shutdown wait
    let mut skip_next_render = false;
    // `dirty` = state changed since the last render. `render_pending` = a
    // frame-tick boundary authorized a render. A redraw happens only when
    // BOTH are true, so no matter how fast tokens arrive the screen refreshes
    // at most at the rate set by `/config` fps (default 10).
    let mut dirty = true;
    let mut render_pending = true;
    // Body cache: a cloned snapshot of the active ChatView, rebuilt at 3 FPS.
    // The spinner (driven by real-time anim_tick) still animates at full frame
    // rate; only the text layout in render_body is throttled.
    let mut body_refresh_pending = true;
    let mut display_chat_cached: Option<ChatView> = None;
    let mut viewport: Option<crate::render_viewport::ViewportCache> = None; // A1: O(visible_h) render cache
                                                                            // Persisted across loop iterations: always equals the LAST rendered
                                                                            // layout (== what is on screen). The event loop forwards `&hits` to
                                                                            // `handle_mouse` on the SAME iteration a click arrives, and a click
                                                                            // sets `dirty=true` so `hits` refreshes next frame. Declaring this
                                                                            // INSIDE the loop resets it to `MouseHits::default()` every turn; when
                                                                            // no render runs (idle state, `dirty=false`) the rects are empty and
                                                                            // EVERY arrow click is silently dropped. Keep this OUTSIDE `loop {}`.
    let mut hits = MouseHits::default();

    // Idle-resize safety net: tracks the last-known terminal (width, height).
    // `frame_ticker` polls the kernel size every frame; if it differs from this
    // (a Resize event lost by crossterm -- tmux, fast window drag) we force a
    // ratatui autoresize + redraw so the screen never lingers at a stale size.
    let mut last_size: Option<(u16, u16)> = terminal.size().ok().map(|r| (r.width, r.height));

    loop {
        app_loop::tick_clock(running, &mut last_clock, &mut run_elapsed_ms);
        let app_loop::DisplayState {
            agent_name,
            status,
            display_chat,
            display_title,
            display_status_agent,
            display_ctx,
            display_sys,
            status_model,
        } = app_loop::compute_display(
            &chat,
            subagent_focus,
            subagent_sys,
            sys_tokens,
            &config,
            &workdir,
        );
        // Refresh the body cache at BODY_REFRESH_MS cadence (3 FPS). Between
        // refreshes the spinner still animates at full frame rate because it is
        // driven by the real-time anim_tick, not the cached blocks.
        if dirty && (body_refresh_pending || display_chat_cached.is_none()) {
            display_chat_cached = Some(display_chat.clone());
            viewport = None; // force viewport rebuild on next render
            body_refresh_pending = false;
        }
        let render_chat = display_chat_cached.as_ref().unwrap_or(display_chat);
        // When a running subagent is focused, show its child view's
        // steer_items in the queue panel instead of the parent's.
        let empty_queue: &[(i64, String)] = &[];
        let (display_steers, display_queue) =
            if let Some(idx) = subagent_focus {
                match chat.blocks.get(idx) {
                    Some(crate::chat::ChatBlock::Subagent {
                        view, done: false, ..
                    }) => (&view.steer_items, empty_queue),
                    _ => (&chat.steer_items, &queue_items[..]),
                }
            } else {
                (&chat.steer_items, &queue_items[..])
            };
        // Input is disabled only when a DONE subagent is focused
        // (not when a running one is — the user can steer it).
        let input_disabled = subagent_focus.is_some_and(|idx| {
            chat.blocks.get(idx).is_none_or(|b| {
                matches!(b, crate::chat::ChatBlock::Subagent { done: true, .. })
            })
        });
        if dirty && render_pending {
            if !skip_next_render {
                app_loop::render_frame(
                    terminal,
                    render_chat,
                    &plan_edit,
                    &input,
                    cursor_idx,
                    &display_title,
                    &display_status_agent,
                    running,
                    show_help,
                    display_ctx,
                    display_sys,
                    context_limit,
                    &status_model,
                    &status,
                    display_steers,
                    display_queue,
                    &mut scroll,
                    follow,
                    anim_tick,
                    &mode_flash,
                    skill_menu.as_ref(),
                    task_picker.as_ref(),
                    command_menu.as_ref(),
                    model_menu.as_ref(),
                    cache_salt_menu.as_ref(),
                    &mut hits,
                    &mut viewport,
                    selection,
                    &copy_status,
                    &pending_images,
                    input_disabled,
                    run_elapsed_ms,
                    help_scroll,
                )?;
            }
            dirty = false;
        }
        render_pending = false;
        skip_next_render = false;
        if quitting {
            break;
        }

        tokio::select! {
            maybe_ev = input_rx.recv() => {
                // `None` ⇒ the input collector thread exited (stdin closed or a
                // read error). Quit instead of busy-looping on a dead source.
                let ev = match maybe_ev {
                    Some(ev) => ev,
                    None => {
                        let _ = cmd_tx.send(UiCmd::Quit).await;
                        break;
                    }
                };
                dirty = true;
                match ev {
                    Event::Key(k) => {
                        copy_status = None;
                        // Plan edit modal: intercept all keys while active.
                        if plan_edit.is_some() {
                            match app_loop::dispatch_plan_edit_key(
                                &mut plan_edit, k, &mut chat, &cmd_tx, terminal,
                            )
                            .await
                            {
                                app_loop::LoopFlow::Quit => break,
                                _ => continue,
                            }
                        }
                        // Task picker modal: intercept all keys while open.
                        if task_picker.is_some() {
                            match handle_task_key(&mut task_picker, k) {
                                TaskOutcome::Pick(pick) => {
                                    app_task::switch_session(
                                        pick,
                                        &mut cmd_tx,
                                        &mut evt_rx,
                                        &workdir,
                                        &config,
                                        &client,
                                        &store,
                                        &mut model_label,
                                        &mut session_states,
                                        &mut running,
                                        &mut chat,
                                        &mut history,
                                        &mut scroll,
                                        &mut follow,
                                        &mut sys_tokens,
                                        &mut queue_items,
                                        &mut active_skill,
                                        &mut active_skill_body,
                                        &mut session_id,
                                        &mut input,
                                        &mut cursor_idx,
                                        &mut hist_idx,
                                        &mut cancel,
                                        &mut skill_handle,
                                    )
                                    .await?;
                                }
                                TaskOutcome::Quit => { let _ = cmd_tx.send(UiCmd::Quit).await; break; }
                                TaskOutcome::ClearAll { keep_session_id } => {
                                    app_task::handle_clear_all(
                                        keep_session_id,
                                        running,
                                        &mut task_picker,
                                        &mut chat,
                                        &store,
                                    )
                                    .await;
                                }
                                TaskOutcome::Idle => {}
                            }
                            continue;
                        }
                        // `/config` modal: intercept all keys while open.
                        if model_menu.is_some() {
                            match app_loop::handle_model_outcome(
                                &mut model_menu, k, &mut client, &mut config, &mut model_label,
                                &mut context_limit, &mut frame_ms, &mut frame_ticker, &cmd_tx,
                                &mut chat, &workdir,
                            )
                            .await
                            {
                                app_loop::LoopFlow::Quit => break,
                                app_loop::LoopFlow::Proceed => {}
                                app_loop::LoopFlow::Redraw => continue,
                            }
                            continue;
                        }
                        // `/cache_salt` read-only panel: intercept all keys while open.
                        if cache_salt_menu.is_some() {
                            match handle_cache_salt_key(&mut cache_salt_menu, k) {
                                CacheSaltOutcome::Quit => {
                                    let _ = cmd_tx.send(UiCmd::Quit).await;
                                    break;
                                }
                                CacheSaltOutcome::Cancel | CacheSaltOutcome::Idle => {}
                            }
                            continue;
                        }
                        // `/` command picker: intercept all keys while open.
                        if command_menu.is_some() {
                            match app_loop::dispatch_command(
                                &mut command_menu, k, &cmd_tx, &mut cancel, &mut chat,
                                &mut running, &mut follow, &store,
                                &session_id, &mut task_picker, &mut model_menu, &config,
                                &mut cache_salt_menu, &agent_name, &mut queue_items,
                            )
                            .await
                            {
                                app_loop::LoopFlow::Quit => break,
                                app_loop::LoopFlow::Proceed => {}
                                app_loop::LoopFlow::Redraw => continue,
                            }
                            continue;
                        }
                        if pre_key_intercept(
                            k,
                            &mut subagent_focus,
                            &mut scroll,
                            &mut follow,
                            &mut selection,
                            &mut last_esc,
                            &mut chat,
                            &mut input,
                            &mut cursor_idx,
                            parent_scroll,
                            parent_follow,
                        ) {
                            continue;
                        }
                        match handle_key(
                            k,
                            &mut input,
                            &mut cursor_idx,
                            &history,
                            &mut hist_idx,
                            running,
                            &agent_name,
                            &mut show_help,
                            &mut scroll,
                            &mut follow,
                            &mut last_esc,
                            &mut skill_menu,
                            // Composer wrap geometry: matches the values used by `render`
                            // (inner_w = term width - 2 borders, prompt_w = 2 for the `❯ ` prefix)
                            // so Up/Down cursor movement tracks the rendered wrapped rows.
                            terminal
                                .size()
                                .map(|r| r.width.saturating_sub(2))
                                .unwrap_or(78),
                            2,
                            subagent_focus.is_some(),
                            input_disabled,
                            &mut undo_state,
                            &mut help_scroll,
                        ) {
                            KeyAction::Submit(text) => {
                                let (clean, _unresolved) = resolve_and_warn(
                                    &text, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir, &skill_handle, &mut chat,
                                );
                                let clean = clean.trim().to_string();
                                if clean.is_empty() {
                                    if active_skill.is_some() {
                                        if !text.is_empty() {
                                            push_user(&mut chat, &mut history, &mut hist_idx, &text);
                                        }
                                        if !running {
                                            // Skill-only submit: send a trigger prompt naming the active
                                            // skill so the model records a user turn and begins acting on
                                            // the skill body injected into the system prompt.
                                            let skill_name = active_skill.as_deref().unwrap_or("");
                                            let trigger = skill_trigger(skill_name);
                                            let image_uris = snapshot_image_uris(&pending_images);
                                            if !start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt(trigger, image_uris)).await
                                            {
                                                worker_dead(&mut chat);
                                                break;
                                            }
                                            pending_images.clear();
                                            running = true;
                                            follow = true;
                                            if chat.agent == "plan" {
                                                chat.plan_submitted = true;
                                            }
                                            chat.begin_turn();
                                        } else {
                                            // Skill-only submit while a turn is running: admit the
                                            // skill trigger as a queued input (mirrors the Queue/Steer
                                            // pure-skill handling) and drain pending images so they
                                            // don't leak into a later unrelated submit.
                                            let skill_name = active_skill.as_deref().unwrap_or("");
                                            let trigger = skill_trigger(skill_name);
                                            let image_uris = snapshot_image_uris(&pending_images);
                                            if let Ok(seq) = store
                                                .admit_input(&mk_input_with_images(&session_id, Delivery::Queue, &trigger, &image_uris))
                                                .await
                                            {
                                                pending_images.clear();
                                                queue_items.push((seq, skill_token_display(skill_name)));
                                            }
                                        }
                                    }
                                } else if running {
                                    let image_uris = snapshot_image_uris(&pending_images);
                                    if let Ok(seq) = store
                                        .admit_input(&mk_input_with_images(&session_id, Delivery::Queue, &clean, &image_uris))
                                        .await
                                    {
                                        pending_images.clear();
                                        queue_items.push((seq, clean.clone()));
                                    }
                                } else {
                                    // Control commands (/act, /plan, /act_clear_context) apply
                                    // without recording a user message — skip the echo so they
                                    // don't appear as literal text in the transcript.
                                    let is_control = opencoder_session::parse_control_cmd(&clean).is_some();
                                    if !is_control {
                                        push_user(&mut chat, &mut history, &mut hist_idx, &text);
                                        chat.context_used += estimate(&clean) as u64;
                                    }
                                    let image_uris = snapshot_image_uris(&pending_images);
                                    if !start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt(clean, image_uris)).await
                                    {
                                        worker_dead(&mut chat);
                                        break;
                                    }
                                    pending_images.clear();
                                    cancelled = false; // B3: clear stale flag from a prior cancel
                                    running = true;
                                    follow = true;
                                    if chat.agent == "plan" {
                                        chat.plan_submitted = true;
                                    }
                                    chat.begin_turn();
                                }
                            }
                            KeyAction::SubagentSteer(text) => {
                                let image_uris = snapshot_image_uris(&pending_images);
                                subagent_input::admit_subagent_steer(
                                    &store,
                                    &mut chat,
                                    subagent_focus,
                                    &text,
                                    &image_uris,
                                )
                                .await;
                                pending_images.clear();
                                follow = true;
                            }
                            KeyAction::Steer(text) => {
                                let (clean, _unresolved) = resolve_and_warn(
                                    &text, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir, &skill_handle, &mut chat,
                                );
                                let clean = clean.trim();
                                if !clean.is_empty() {
                                    let image_uris = snapshot_image_uris(&pending_images);
                                    if let Ok(seq) = store.admit_input(&mk_input_with_images(&session_id, Delivery::Steer, clean, &image_uris)).await {
                                        pending_images.clear();
                                        chat.steer_items.push((seq, clean.to_string()));
                                    }
                                    // Do NOT echo into the main transcript /
                                    // execution area. Steer input is surfaced
                                    // only in the side queue panel + status bar
                                    // badge, consistent with queued inputs.
                                } else if let Some(skill_name) = active_skill.as_deref() {
                                    // Pure-skill submit (only a `{$name}` token,
                                    // no text): admit the skill trigger as a
                                    // steer so the skill body — already injected
                                    // into the system prompt — is acted on via
                                    // the steer queue rather than being dropped.
                                    let trigger = skill_trigger(skill_name);
                                    let image_uris = snapshot_image_uris(&pending_images);
                                    if let Ok(seq) = store.admit_input(&mk_input_with_images(&session_id, Delivery::Steer, &trigger, &image_uris)).await {
                                        pending_images.clear();
                                        chat.steer_items.push((seq, skill_token_display(skill_name)));
                                    }
                                }
                                follow = true;
                            }
                            KeyAction::Queue(text) => {
                                let (clean, _unresolved) = resolve_and_warn(
                                    &text, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir, &skill_handle, &mut chat,
                                );
                                let clean = clean.trim();
                                if !clean.is_empty() {
                                    let image_uris = snapshot_image_uris(&pending_images);
                                    if let Ok(seq) = store.admit_input(&mk_input_with_images(&session_id, Delivery::Queue, clean, &image_uris)).await {
                                        pending_images.clear();
                                        queue_items.push((seq, clean.to_string()));
                                    }
                                } else if let Some(skill_name) = active_skill.as_deref() {
                                    // Pure-skill submit (only a `{$name}` token,
                                    // no text): admit the skill trigger to the
                                    // queue so the active skill is acted on
                                    // instead of being silently dropped.
                                    let trigger = skill_trigger(skill_name);
                                    let image_uris = snapshot_image_uris(&pending_images);
                                    if let Ok(seq) = store.admit_input(&mk_input_with_images(&session_id, Delivery::Queue, &trigger, &image_uris)).await {
                                        pending_images.clear();
                                        queue_items.push((seq, skill_token_display(skill_name)));
                                    }
                                }
                                follow = true;
                            }
                            KeyAction::QueueUnsupported => {
                                // Tab-queue was rejected because a running
                                // subagent is focused: show a transient hint
                                // and do NOT touch the parent session.
                                mode_flash = Some((
                                    "\u{26a0} tab queue not supported for subagents \u{2014} press Enter to steer".into(),
                                    anim_tick,
                                ));
                            }
                            KeyAction::SwitchAgent(name) => {
                                if matches!(
                                    app_loop::handle_switch_agent(
                                        name, &mut chat, &mut running, &mut follow, &mut input,
                                        &mut cursor_idx, &mut mode_flash,
                                        anim_tick, &cmd_tx, &mut cancel, &mut sys_tokens,
                                        &workdir, &active_skill_body,
                                    )
                                    .await,
                                    app_loop::SwitchOutcome::Quit
                                ) {
                                    break;
                                }
                            }
                            KeyAction::SwitchAgentNoClear(name) => {
                                // t+Tab chord: switch agent mode but skip the
                                // plan->act handoff / TranscriptReset — the
                                // transcript is preserved in full, unlike
                                // Shift+Tab which collapses to the final plan.
                                mode_flash = Some((format!("\u{2192} {name} mode"), anim_tick));
                                sys_tokens =
                                    sys_tokens_for(&name, &workdir, active_skill_body.as_deref());
                                let _ = cmd_tx.send(UiCmd::SwitchAgent(name)).await;
                            }
                            KeyAction::SetSkill(opt) => {
                                let skill_body = opt.as_ref().map(|(_, body)| body.clone());
                                match opt {
                                    Some((name, body)) => {
                                        active_skill = Some(name.clone());
                                        active_skill_body = Some(body.clone());
                                        sys_tokens = sys_tokens_for(&agent_name, &workdir, Some(&body));
                                        *skill_handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(body);
                                    }
                                    None => {
                                        active_skill = None;
                                        active_skill_body = None;
                                        sys_tokens = sys_tokens_for(&agent_name, &workdir, None);
                                        *skill_handle.lock().unwrap_or_else(|e| e.into_inner()) = None;
                                    }
                                }
                                // Persist the active skill so it survives
                                // resume/restart (best-effort; the in-memory
                                // mutex write above keeps the in-flight turn
                                // immediate).
                                let _ = store
                                    .update_session(
                                        &session_id,
                                        &opencoder_store::SessionPatch {
                                            skill: skill_body,
                                            updated_at: Some(opencoder_core::message::now_ms()),
                                            ..Default::default()
                                        },
                                    )
                                    .await;
                            }
                            KeyAction::Cancel => {
                                cancel.cancel();
                                // Double-Esc hard-abort: also drop any pending
                                // steer/queue inputs so they don't resurface on
                                // resume. delete_input is idempotent.
                                clear_pending_inputs(
                                    store.as_ref(),
                                    &mut chat.steer_items,
                                    &mut queue_items,
                                )
                                .await;
                                chat.push_marker(Line::from(Span::styled(
                                    "[interrupted] stopping…", Style::default().fg(Color::Yellow))));
                                running = false;
                                cancelled = true;
                                follow = true;
                            }
                            KeyAction::EnterPlanEdit => {
                                app_loop::enter_plan_edit(
                                    &mut plan_edit, &chat, &mut mode_flash, anim_tick,
                                );
                            }
                            KeyAction::OpenCommand => {
                                command_menu = Some(CommandMenu::new());
                            }
                            KeyAction::Quit => {
                                app_loop::handle_quit(running, &cancel, &mut chat, &cmd_tx).await;
                                chat.status = "shutting down\u{2026}".to_string();
                                dirty = true;
                                render_pending = true;
                                quitting = true;
                            }
                            KeyAction::Clip => {
                                app_loop::paste_clipboard_image(
                                    &mut chat, &mut pending_images,
                                )
                                .await;
                                dirty = true;
                            }
                            KeyAction::None => {}
                        }
                    }
                    Event::Mouse(m) => {
                        let mut copy_msg: Option<String> = None;
                        let outcome = handle_mouse(
                            m, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
                            &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
                            &mut subagent_sys, &workdir, &mut queue_items, &session_id,
                            store.as_ref(), &mut copy_msg, &mut last_click, &mut dbl_click,
                        )
                        .await;
                        if let Some(msg) = copy_msg {
                            copy_status = Some((msg, Instant::now()));
                        }
                        if outcome == MouseOutcome::SteerSubmit {
                            // When a running subagent is focused, fire its
                            // turn-cancel to interrupt the current turn and
                            // force immediate steer absorption. Do NOT change
                            // the subagent's status — it continues running.
                            let sub_focused = subagent_focus.is_some();
                            if sub_focused {
                                subagent_input::fire_subagent_turn_cancel(
                                    &child_turn_cancels,
                                    &chat,
                                    subagent_focus,
                                );
                            } else if running {
                                cancel.cancel();
                                cancelled = true;
                                drain_pending = true;
                            } else {
                                start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt(String::new(), Vec::new()))
                                    .await;
                                running = true;
                                chat.begin_turn();
                            }
                            follow = true;
                        }
                    }
                    Event::Resize(_, _) => on_resize_event(terminal, &mut last_size),
                    Event::Paste(pasted) => {
                        // Modal-priority paste routing (mirrors Event::Key).
                        if let app_loop::LoopFlow::Redraw = app_loop::route_paste(
                            &pasted, task_picker.is_some(), cache_salt_menu.is_some(),
                            &mut model_menu, &mut command_menu, &mut input,
                            &mut cursor_idx, &mut pending_images, &workdir,
                        ) {
                            continue;
                        }
                    }
                    _ => {}
                }
            }
            maybe_ev = evt_rx.recv() => {
                match app_loop::fold_ui_events(
                    maybe_ev, &mut chat, &store, &session_id, &mut queue_items, &mut running,
                    &mut cancelled, &mut drain_pending, &mut skip_next_render, &mut follow,
                    &cmd_tx, &mut cancel, &mut evt_rx,
                )
                .await
                {
                    app_loop::LoopFlow::Quit => break,
                    app_loop::LoopFlow::Proceed => dirty = true,
                    app_loop::LoopFlow::Redraw => continue,
                }
            }
            _ = anim_ticker.tick() => {
                if running {
                    anim_tick = anim_tick.wrapping_add(1);
                    dirty = true;
                }
            }
            _ = frame_ticker.tick() => {
                render_pending = true;
                if poll_idle_resize(terminal, &mut last_size) {
                    dirty = true;
                }
            }
            _ = body_ticker.tick() => {
                body_refresh_pending = true;
            }
        }
    }

    // Disarm the liveness supervisor: once we drop the input pump its heartbeat
    // stops advancing, which is expected during shutdown — not a wedge.
    supervisor_active.store(false, Ordering::Relaxed);
    drop(cmd_tx);
    // The cancel issued on Quit should make the worker finish promptly. As a
    // last-resort guard against a tool/subagent that ignores cancellation,
    // bound the wait so the terminal is restored (TerminalGuard::drop leaves
    // the alt-screen) instead of freezing indefinitely on a blocked worker.
    let _ = tokio::time::timeout(Duration::from_secs(5), worker).await;
    Ok(session_id)
}

pub(crate) use crate::app_helpers::{
    clear_pending_inputs, handle_mouse, initial_chat_view, mk_input_with_images, on_resize_event,
    poll_idle_resize, pre_key_intercept, push_user, resolve_and_warn, snapshot_image_uris,
    start_turn, sys_tokens_for, worker_dead, MouseOutcome,
};
pub(crate) use crate::skill_display::{skill_token_display, skill_trigger};

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
