use anyhow::Result;
use crossterm::event::Event;
use opencoder_core::Config;
use opencoder_llm::{estimate, ChatStream};
use opencoder_session::SessionState;
use opencoder_store::{Delivery, Store};
use ratatui::{
    style::Style,
    text::{Line, Span},
};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
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
use crate::skill_persist::resolve_persist;
use crate::task::{handle_task_key, TaskOutcome, TaskPicker};
use crate::terminal::consume_modifier_or_release;
use crate::theme;
use crate::worker::{process_cmd, UiCmd, UiEvent};
use crate::TuiOpts;
#[path = "app_loop.rs"]
pub(crate) mod app_loop;

#[path = "app_bootstrap.rs"]
mod app_bootstrap;
#[path = "app_display.rs"]
mod app_display;
#[path = "app_task.rs"]
mod app_task;

#[path = "steer_dispatch.rs"]
mod steer_dispatch;
#[path = "steer_fire.rs"]
mod steer_fire;
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
    mut compaction_threshold: u64,
    mut context_limit: u64,
    mut model_label: String,
    workdir: PathBuf,
    mut config: Config,
    mut client: Arc<dyn ChatStream>,
) -> Result<String> {
    // Cancellation token for double-Esc hard-abort (mid-stream/mid-tool).
    // Reassigned by `rebind_session` on every `/task` session switch.
    let mut cancel = CancellationToken::new();
    let session = session.with_cancel(cancel.clone());
    // Parent turn-level interrupt: TUI `>` steer fires this (not a hard `cancel`) so a
    // pending steer is absorbed at the next boundary; the run loop continues, unlike
    // double-Esc. Reassigned by `rebind_session` on every `/task` switch.
    let mut turn_cancel = session
        .turn_cancel
        .clone()
        .unwrap_or_else(|| Arc::new(std::sync::Mutex::new(CancellationToken::new())));
    let mut child_runtime = crate::worker::ChildRuntimeHandles::from_session(&session);
    let mut skill_handle = session.skill_prompt.clone();
    let mut chat = initial_chat_view(&session, &store).await;
    chat.requirement_text = session.requirement.clone();
    let mut input = String::new();
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut img_asm = crate::image_chunk::Assembly::new();
    let mut cursor_idx: usize = 0;
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut running = false;
    let mut prev_running = false;
    let mut task_elapsed_ms: u64 = 0;
    let (mut last_clock, mut cancelled) = (Instant::now(), false);
    let mut drain_pending = false;
    let mut undo_state = crate::undo::init(&input, cursor_idx);
    let mut scroll: u32 = 0;
    let mut follow = true;
    // Queue/steer panel scroll offset (0 = pinned to top (oldest)); snapshot/restored per-session.
    let mut queue_scroll: u32 = 0;
    let mut plan_edit: Option<crate::plan_edit::PlanEdit> = None;
    let initial_skill_body = skill_handle.lock().ok().and_then(|g| g.clone());
    let mut sys_tokens: u64 = sys_tokens_for(
        session.agent.name.as_str(),
        &workdir,
        initial_skill_body.as_deref(),
    );
    // Cached system-prompt tokens for the subagent currently being viewed.
    // Computed once on entry (ctx-switch click) to avoid per-frame rebuild.
    let mut subagent_sys: u64 = 0;
    let mut queue_items =
        crate::queue_panel::restore_pending_mirrors(&store, &session_id, &mut chat.steer_items)
            .await;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut task_picker: Option<TaskPicker> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut model_menu: Option<ModelMenu> = None;
    let mut cache_salt_menu: Option<CacheSaltMenu> = None;
    let mut keymap_menu: Option<crate::keymap_menu::KeymapMenu> = None;
    let mut keymap = crate::keymap::KeyBindings::from_config(&config);
    let mut active_skill: Option<String> = None;
    let mut active_skill_body: Option<String> = None;
    let mut anim_tick: u32 = 0;
    let mut mode_flash: Option<(String, u32)> = None;
    let mut last_esc: Option<Instant> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut shift_held = false;
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

    // Input is collected on a dedicated OS thread and delivered over `input_rx`.
    // Liveness supervisor: when crossterm 0.28's mio source busy-loops on pty
    // close (holding the event mutex so `poll` never returns), a separate
    // supervisor thread detects the stall, restores the terminal, exits cleanly.
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
    let mut dirty = true;
    let mut render_pending = true;
    let mut body_refresh_pending = true;
    let mut display_chat_cached: Option<ChatView> = None;
    let mut viewport: Option<crate::render_viewport::ViewportCache> = None;
    let mut hits = MouseHits::default();

    let mut last_size: Option<(u16, u16)> = terminal.size().ok().map(|r| (r.width, r.height));
    loop {
        app_loop::tick_clock(
            running,
            &mut prev_running,
            &mut last_clock,
            &mut task_elapsed_ms,
        );
        let app_loop::DisplayState {
            agent_name,
            display_mode,
            status,
            display_chat,
            display_title,
            display_ctx,
            display_sys,
        } = app_loop::compute_display(
            &chat,
            subagent_focus,
            subagent_sys,
            sys_tokens,
            &config,
            &workdir,
            last_size.map_or(0, |(w, _)| w),
            u16::from(scroll > 0) * app_display::TOP_ARROW_W,
        );
        if dirty && (body_refresh_pending || display_chat_cached.is_none()) {
            display_chat_cached = Some(display_chat.clone());
            viewport = None; // force viewport rebuild on next render
            body_refresh_pending = false;
        }
        let render_chat = display_chat_cached.as_ref().unwrap_or(display_chat);
        let (display_steers, display_queue) =
            app_display::steer_queue_sources(&chat, subagent_focus, &queue_items);
        let input_disabled = app_display::is_input_disabled(&chat, subagent_focus);
        let now = opencoder_core::message::now_ms();
        let tail_ms = app_display::display_tail_ms(&chat, subagent_focus, now, running);

        if dirty && render_pending {
            if !skip_next_render {
                app_loop::render_frame(
                    terminal,
                    render_chat,
                    &plan_edit,
                    &input,
                    cursor_idx,
                    &display_title,
                    running,
                    display_ctx,
                    display_sys,
                    compaction_threshold,
                    context_limit,
                    &status,
                    display_steers,
                    display_queue,
                    &mut scroll,
                    follow,
                    &mut queue_scroll,
                    anim_tick,
                    now,
                    &mode_flash,
                    skill_menu.as_ref(),
                    task_picker.as_ref(),
                    command_menu.as_ref(),
                    model_menu.as_ref(),
                    cache_salt_menu.as_ref(),
                    keymap_menu.as_ref(),
                    &mut hits,
                    &mut viewport,
                    shift_held,
                    &pending_images,
                    input_disabled,
                    tail_ms,
                    task_elapsed_ms,
                    subagent_focus.is_none(),
                    config.autopilot.enabled, &display_mode,
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
            biased;
            maybe_ev = input_rx.recv() => {
                // `None` ⇒ the input collector thread exited (stdin closed/read error); quit instead of busy-looping.
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
                        if consume_modifier_or_release(&k, &mut shift_held) {
                            dirty = true;
                            continue;
                        }
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
                                        terminal,
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
                                        &mut queue_scroll,
                                        &mut sys_tokens,
                                        &mut queue_items,
                                        &mut active_skill,
                                        &mut active_skill_body,
                                        &mut session_id,
                                        &mut input,
                                        &mut cursor_idx,
                                        &mut hist_idx,
                                        &mut cancel,
                                        &mut turn_cancel,
                                        &mut child_runtime,
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
                        if model_menu.is_some() {
                            match app_loop::handle_model_outcome(&mut model_menu, k, &mut client, &mut config, &mut model_label,
                                &mut compaction_threshold, &mut context_limit, &mut frame_ms, &mut frame_ticker, &cmd_tx, &mut chat, &workdir).await {
                                app_loop::LoopFlow::Quit => break, app_loop::LoopFlow::Redraw => continue, _ => {}
                            }
                            continue;
                        }
                        if cache_salt_menu.is_some() {
                            if matches!(handle_cache_salt_key(&mut cache_salt_menu, k), CacheSaltOutcome::Quit) { let _ = cmd_tx.send(UiCmd::Quit).await; break; }
                            continue;
                        }
                        if keymap_menu.is_some() {
                            if let app_loop::LoopFlow::Quit = app_loop::handle_keymap_outcome(&mut keymap_menu, k, &mut config, &mut keymap, &workdir, &cmd_tx).await { break }
                            dirty = true; render_pending = true; continue;
                        }
                        // `/` command picker: intercept all keys while open.
                        if command_menu.is_some() {
                            match app_loop::dispatch_command(
                                &mut command_menu, k, &cmd_tx, &mut cancel, &mut chat,
                                &mut running, &mut follow, &store,
                                &session_id, &mut task_picker, &mut model_menu,
                                &mut cache_salt_menu, &mut keymap_menu, &agent_name,
                                &mut input, &mut cursor_idx,
                                &mut config, &workdir,
                                &mut mode_flash, anim_tick, &mut sys_tokens,
                                &mut plan_edit,
                            )
                            .await
                            {
                                app_loop::LoopFlow::Quit => break,
                                app_loop::LoopFlow::Proceed => {}
                                app_loop::LoopFlow::InstallTools => {
                                    crate::install_tools::run(terminal, &mut chat);
                                    dirty = true;
                                    render_pending = true;
                                    continue;
                                }
                                app_loop::LoopFlow::Redraw => continue,
                            }
                            continue;
                        }
                        let mut needs_clear = false;
                        if pre_key_intercept(
                            k,
                            &keymap,
                            &mut subagent_focus,
                            &mut follow,
                            &mut last_esc,
                            &mut chat,
                            &mut input,
                            &mut cursor_idx,
                            &mut needs_clear,
                        ) {
                            apply_force_redraw(
                                needs_clear,
                                &mut *terminal,
                                &mut render_pending,
                                &mut skip_next_render,
                            );
                            continue;
                        }
                        match handle_key(
                            k,
                            &keymap,
                            &mut input,
                            &mut cursor_idx,
                            &history,
                            &mut hist_idx,
                            running,
                            &agent_name,
                            &mut scroll,
                            &mut follow,
                            &mut last_esc,
                            &mut skill_menu,
                            // Composer wrap geometry matches `render` (inner_w = width-2, prompt_w = 2 for `❯ `),
                            // so Up/Down cursor movement tracks the rendered wrapped rows.
                            terminal
                                .size()
                                .map(|r| r.width.saturating_sub(2))
                                .unwrap_or(78),
                            2,
                            subagent_focus.is_some(),
                            input_disabled,
                            &mut undo_state,
                            &mut queue_scroll,
                        ) {
                            KeyAction::Submit(text) => {
                                let (clean, _unresolved) = resolve_persist(
                                    &text, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir, &skill_handle, &mut chat,
                                    &store, &session_id,
                                ).await;
                                let clean = clean.trim().to_string();
                                let clean = crate::control_helpers::forward_skill_if_compound(&text, &clean);
                                // A compound `/plan <content>` delivered while the agent is still
                                // `act` arms the plan->act handoff *deferred*: the mode switch
                                // lands asynchronously via `AgentSwitch("plan")`, which would
                                // reset `plan_submitted`, so leave a pending flag for that event
                                // to re-arm. Shift+Tab after the plan turn then keeps the plan
                                // and starts the task instead of plain-swapping.
                                if chat.agent != "plan" && crate::control_helpers::is_compound_plan_cmd(&clean) { chat.pending_plan_arm = true; }
                                // Intercept /requirement: open the editor instead of submitting
                                if clean == "/requirement" {
                                    crate::plan_edit::enter_requirement(
                                        &mut plan_edit,
                                        chat.last_requirement_text().unwrap_or_default(),
                                    );
                                    mode_flash = Some(("\u{2192} requirement".into(), anim_tick));
                                } else if crate::local_cmd::run(&clean, &mut chat, &mut config, &cmd_tx, &workdir).await { // /ps /stop /ap
                                } else if clean.is_empty() {
                                    if active_skill.is_some() {
                                        if !text.is_empty() {
                                            push_user(&mut chat, &mut history, &mut hist_idx, &text);
                                        }
                                        if !running {
                                            // Skill-only submit: send a trigger prompt naming the active skill so
                                            // the model records a user turn and acts on the injected skill body.
                                            let skill_name = active_skill.as_deref().unwrap_or("");
                                            let trigger = skill_trigger(skill_name);
                                            let image_uris = snapshot_image_uris(&pending_images);
                                            if !start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt(trigger, image_uris)).await
                                            {
                                                worker_dead(&mut chat);
                                                break;
                                            }
                                            pending_images.clear();
                                            task_elapsed_ms = 0;
                                            running = true;
                                            follow = true;
                                            chat.note_requirement_submitted();
                                            chat.begin_turn();
                                        } else {
                                            // Skill-only submit while running: admit the trigger as a queued input and
                                            // drain pending images so they don't leak into a later unrelated submit.
                                            let skill_name = active_skill.as_deref().unwrap_or("");
                                            let trigger = skill_trigger(skill_name);
                                            let image_uris = snapshot_image_uris(&pending_images);
                                            if let Ok(seq) = store
                                                .admit_input(&mk_input_with_images(&session_id, Delivery::Queue, &trigger, Some(skill_token_display(skill_name)), &image_uris))
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
                                        .admit_input(&mk_input_with_images(&session_id, Delivery::Queue, &clean, Some(queued_item_display(&text, &clean)), &image_uris))
                                        .await
                                    {
                                        pending_images.clear();
                                        queue_items.push((seq, queued_item_display(&text, &clean)));
                                    }
                                } else {
                                    // Only suppress the transcript echo for BARE control commands
                                    // (/plan, /act, /act_clear_context). Compound inputs
                                    // (/plan $review, /plan fix the bug) carry user content and
                                    // must be echoed before execution.
                                    let is_pure_control = crate::control_helpers::is_pure_control_cmd(&clean);
                                    if !is_pure_control {
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
                                    task_elapsed_ms = 0;
                                    cancelled = false; // B3: clear stale flag from a prior cancel
                                    running = true;
                                    follow = true;
                                    chat.note_requirement_submitted();
                                    chat.begin_turn();
                                }
                            }
                            KeyAction::SubagentSteer(text) => {
                                subagent_input::handle_subagent_steer(&store, &child_runtime.steer_gates, &mut chat, subagent_focus, text, &mut pending_images, &mut input, &mut cursor_idx).await;
                                follow = true;
                            }
                            KeyAction::Steer(text) => {
                                let (clean, _unresolved) = resolve_persist(
                                    &text, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir, &skill_handle, &mut chat,
                                    &store, &session_id,
                                ).await;
                                let clean = clean.trim();
                                let clean = crate::control_helpers::forward_skill_if_compound(&text, clean);
                                if chat.agent != "plan" && crate::control_helpers::is_compound_plan_cmd(&clean) { chat.pending_plan_arm = true; }
                                if !clean.is_empty() {
                                    let display = queued_item_display(&text, &clean);
                                    steer_fire::admit_keyboard_steer(
                                        &store, &session_id, &clean, &display,
                                        &mut pending_images, &mut chat,
                                    )
                                    .await;
                                } else if let Some(skill_name) = active_skill.as_deref() {
                                    // Pure-skill submit (only a `$name` token): admit the trigger
                                    // as a steer so the injected skill body is acted on, not dropped.
                                    let trigger = skill_trigger(skill_name);
                                    let display = skill_token_display(skill_name);
                                    steer_fire::admit_keyboard_steer(
                                        &store, &session_id, &trigger, &display,
                                        &mut pending_images, &mut chat,
                                    )
                                    .await;
                                }
                                push_history(&mut history, &mut hist_idx, &text);
                                // Enter admits without interrupting (`>` interrupts instead).
                                follow = true;
                            }
                            KeyAction::Queue(text) => {
                                let (clean, _unresolved) = resolve_persist(
                                    &text, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir, &skill_handle, &mut chat,
                                    &store, &session_id,
                                ).await;
                                let clean = clean.trim();
                                let clean = crate::control_helpers::forward_skill_if_compound(&text, clean);
                                if chat.agent != "plan" && crate::control_helpers::is_compound_plan_cmd(&clean) { chat.pending_plan_arm = true; }
                                if !clean.is_empty() {
                                    let display = queued_item_display(&text, &clean);
                                    let image_uris = snapshot_image_uris(&pending_images);
                                    if let Ok(seq) = store.admit_input(&mk_input_with_images(&session_id, Delivery::Queue, &clean, Some(display.clone()), &image_uris)).await {
                                        pending_images.clear();
                                        queue_items.push((seq, display.clone()));
                                        chat.note_requirement_submitted();
                                    }
                                } else if let Some(skill_name) = active_skill.as_deref() {
                                    // Pure-skill submit: admit the trigger so the active skill is acted on.
                                    let trigger = skill_trigger(skill_name);
                                    let display = skill_token_display(skill_name);
                                    let image_uris = snapshot_image_uris(&pending_images);
                                    if let Ok(seq) = store.admit_input(&mk_input_with_images(&session_id, Delivery::Queue, &trigger, Some(display.clone()), &image_uris)).await {
                                        pending_images.clear();
                                        queue_items.push((seq, display.clone()));
                                        chat.note_requirement_submitted();
                                    }
                                }
                                push_history(&mut history, &mut hist_idx, &text);
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
                                        name, false, &mut chat, &mut running, &mut follow, &mut input,
                                        &mut cursor_idx, &mut mode_flash, anim_tick, &cmd_tx,
                                        &mut cancel, &mut sys_tokens, &workdir, &active_skill_body,
                                    )
                                    .await,
                                    app_loop::SwitchOutcome::Quit
                                ) { break; }
                            }
                            KeyAction::SwitchAgentNoClear(name) => {
                                // t+Tab chord: skip the plan->act handoff / TranscriptReset —
                                // transcript preserved in full. Same running-gated handler as
                                // Shift+Tab so a mode switch mid-turn defers to the next idle
                                // boundary (no direct UiCmd::SwitchAgent leak).
                                if matches!(
                                    app_loop::handle_switch_agent(
                                        name, true, &mut chat, &mut running, &mut follow, &mut input,
                                        &mut cursor_idx, &mut mode_flash, anim_tick, &cmd_tx,
                                        &mut cancel, &mut sys_tokens, &workdir, &active_skill_body,
                                    )
                                    .await,
                                    app_loop::SwitchOutcome::Quit
                                ) { break; }
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
                                // Persist the active skill (best-effort) so it survives resume/restart;
                                // the in-memory mutex write above keeps the in-flight turn immediate.
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
                                // A cancelled compound `/plan` never reaches the runner's
                                // AgentSwitch, so drop the deferred arming it left behind.
                                chat.pending_plan_arm = false;
                                cancel.cancel();
                                opencoder_session::fire_child_cancels(&child_runtime.cancels);
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
                                    "[interrupted] stopping…", Style::default().fg(theme::warn_color()))));
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
                            crate::key_handler::KeyAction::OpenKeymap => {
                                keymap_menu = Some(crate::keymap_menu::KeymapMenu::new(&config.keymap));
                                dirty = true;
                                render_pending = true;
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
                        let outcome = handle_mouse(
                            m, &hits, &mut scroll, &mut follow, &mut chat,
                            &mut subagent_focus,
                            &mut subagent_sys, &workdir, &mut queue_items, &session_id,
                            store.as_ref(), &mut queue_scroll,
                        )
                        .await;
                        if outcome == MouseOutcome::SteerSubmit {
                            let outcome = steer_fire::handle_steer_submit(
                                subagent_focus, running, &child_runtime.cancels,
                                &child_runtime.turn_cancels, &turn_cancel, &chat,
                            );
                            if outcome == steer_fire::SteerSubmitOutcome::StartTurn {
                                start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt(String::new(), Vec::new())).await;
                                running = true;
                                chat.begin_turn();
                            }
                            follow = true;
                        }
                        dirty = true;
                    }
                    Event::Resize(_, _) => on_resize_event(terminal, &mut last_size)?,
                    Event::Paste(pasted) => {
                        if pasted.trim().is_empty() {
                            app_loop::paste_clipboard_image_silent(&mut chat, &mut pending_images).await;
                            dirty = true;
                            continue;
                        }
                        // Modal-priority paste routing (mirrors Event::Key).
                        if let app_loop::LoopFlow::Redraw = app_loop::route_paste(
                            &pasted, task_picker.is_some(), cache_salt_menu.is_some(), keymap_menu.is_some(),
                            &mut model_menu, &mut command_menu, &mut input,
                            &mut cursor_idx, &mut pending_images, &mut img_asm,
                            &mut chat, &workdir,
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
                    app_loop::LoopFlow::InstallTools => dirty = true,
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
                if poll_idle_resize(terminal, &mut last_size)? {
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
    // Last-resort guard against a tool/subagent ignoring the Quit cancel: bound
    // the worker wait so the terminal is restored instead of freezing on a blocked worker.
    let _ = tokio::time::timeout(Duration::from_secs(5), worker).await;
    Ok(session_id)
}

pub(crate) use crate::app_helpers::{
    apply_force_redraw, clear_pending_inputs, handle_mouse, initial_chat_view,
    mk_input_with_images, on_resize_event, poll_idle_resize, pre_key_intercept, push_history,
    push_user, snapshot_image_uris, start_turn, sys_tokens_for, worker_dead, MouseOutcome,
};
pub(crate) use crate::skill_display::{queued_item_display, skill_token_display, skill_trigger};

#[cfg(test)]
#[path = "app_tests/mod.rs"]
mod tests;
