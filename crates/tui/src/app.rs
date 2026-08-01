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
use ratatui::style::Style;
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
use crate::skill_persist::resolve_persist;
use crate::task::{handle_task_key, TaskOutcome, TaskPicker};
use crate::theme;
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

#[path = "app_display.rs"]
mod app_display;

#[path = "steer_dispatch.rs"]
mod steer_dispatch;

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
    // Parent turn-level interrupt handle. The TUI `>` steer button fires
    // this (instead of a hard `cancel`) to interrupt the parent's current
    // LLM/tool turn so a pending steer is absorbed at the next boundary --
    // the run loop continues rather than aborting like double-Esc.
    // Reassigned by `rebind_session` on every `/task` switch.
    let mut turn_cancel = session.turn_cancel.clone().unwrap_or_else(|| {
        Arc::new(std::sync::Mutex::new(CancellationToken::new()))
    });
    let child_turn_cancels = session.child_turn_cancels.clone();
    let child_cancels = session.child_cancels.clone();
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
    let initial_skill_body = skill_handle.lock().ok().and_then(|g| g.clone());
    let mut sys_tokens: u64 =
        sys_tokens_for(session.agent.name.as_str(), &workdir, initial_skill_body.as_deref());
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
    // Transient copy-feedback (~2s) after a mouse-drag copy. Uses `Instant`
    // (not `anim_tick`, which only advances while running) so idle copies expire.
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
    // `dirty` = state changed since last render; `render_pending` = a frame tick
    // authorized it. Redraw needs BOTH, capping refresh at `/config` fps (default 10).
    let mut dirty = true;
    let mut render_pending = true;
    // Body cache: a cloned snapshot of the active ChatView, rebuilt at 3 FPS.
    // The spinner (driven by real-time anim_tick) still animates at full frame
    // rate; only the text layout in render_body is throttled.
    let mut body_refresh_pending = true;
    let mut display_chat_cached: Option<ChatView> = None;
    let mut viewport: Option<crate::render_viewport::ViewportCache> = None;
    // `hits` persists outside `loop {}` — resetting inside drops idle clicks.
    let mut hits = MouseHits::default();

    // Idle-resize safety net: `frame_ticker` polls the kernel size each frame;
    // on mismatch (a lost Resize event — tmux, fast drag) we force autoresize + redraw.
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
        if dirty && (body_refresh_pending || display_chat_cached.is_none()) {
            display_chat_cached = Some(display_chat.clone());
            viewport = None; // force viewport rebuild on next render
            body_refresh_pending = false;
        }
        let render_chat = display_chat_cached.as_ref().unwrap_or(display_chat);
        let (display_steers, display_queue) =
            app_display::steer_queue_sources(&chat, subagent_focus, &queue_items);
        let input_disabled = app_display::is_input_disabled(&chat, subagent_focus);

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
                    compaction_threshold,
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
                    subagent_focus.is_none(),
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
                                &mut compaction_threshold, &mut context_limit, &mut frame_ms,
                                &mut frame_ticker, &cmd_tx, &mut chat, &workdir,
                            )
                            .await
                            {
                                app_loop::LoopFlow::Quit => break,
                                app_loop::LoopFlow::Proceed => {}
                                app_loop::LoopFlow::InstallTools => {}
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
                                &mut cache_salt_menu, &agent_name,
                                &mut input, &mut cursor_idx,
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
                                let (clean, _unresolved) = resolve_persist(
                                    &text, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir, &skill_handle, &mut chat,
                                    &store, &session_id,
                                ).await;
                                let clean = clean.trim().to_string();
                                if crate::local_cmd::run(&clean, &mut chat) { // /ps /stop: display-only
                                } else if clean.is_empty() {
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
                                            // Skill-only submit while running: admit the skill trigger
                                            // as a queued input and drain pending images so they don't
                                            // leak into a later unrelated submit.
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
                                let (clean, _unresolved) = resolve_persist(
                                    &text, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir, &skill_handle, &mut chat,
                                    &store, &session_id,
                                ).await;
                                let clean = clean.trim();
                                if !clean.is_empty() {
                                    let image_uris = snapshot_image_uris(&pending_images);
                                    if let Ok(seq) = store.admit_input(&mk_input_with_images(&session_id, Delivery::Steer, clean, &image_uris)).await {
                                        pending_images.clear();
                                        chat.steer_items.push((seq, clean.to_string()));
                                    }
                                    // Steer input isn't echoed in the transcript; it's surfaced only
                                    // in the side queue panel + status bar badge (like queued inputs).
                                } else if let Some(skill_name) = active_skill.as_deref() {
                                    // Pure-skill submit (only a `{$name}` token): admit the trigger
                                    // as a steer so the injected skill body is acted on, not dropped.
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
                                let (clean, _unresolved) = resolve_persist(
                                    &text, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir, &skill_handle, &mut chat,
                                    &store, &session_id,
                                ).await;
                                let clean = clean.trim();
                                if !clean.is_empty() {
                                    let image_uris = snapshot_image_uris(&pending_images);
                                    if let Ok(seq) = store.admit_input(&mk_input_with_images(&session_id, Delivery::Queue, clean, &image_uris)).await {
                                        pending_images.clear();
                                        queue_items.push((seq, clean.to_string()));
                                    }
                                } else if let Some(skill_name) = active_skill.as_deref() {
                                    // Pure-skill submit (only a `{$name}` token): admit the trigger
                                    // to the queue so the active skill is acted on, not dropped.
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
                                // t+Tab chord: switch agent mode but skip the plan->act handoff /
                                // TranscriptReset — transcript preserved in full (Shift+Tab collapses it).
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
                            let sub_focused = subagent_focus.is_some();
                            // fire_child_cancels both checks AND cancels children — only call it
                            // when the parent is running and no subagent row is focused.
                            let has_children = !sub_focused && running
                                && opencoder_session::fire_child_cancels(&child_cancels);
                            match steer_dispatch::resolve(
                                sub_focused, running, has_children, !chat.steer_items.is_empty(),
                            ) {
                                steer_dispatch::Action::Subagent => {
                                    subagent_input::fire_subagent_turn_cancel(
                                        &child_turn_cancels, &chat, subagent_focus,
                                    );
                                }
                                steer_dispatch::Action::CancelChildren => {
                                    // Children cancelled — run_loop absorbs the steer at the next
                                    // turn boundary via err("cancelled") tool results.
                                }
                                steer_dispatch::Action::SteerParent => {
                                    // G1: no running children but a steer is pending — interrupt the
                                    // parent's current LLM/tool turn (soft cancel, not hard abort).
                                    opencoder_session::fire_turn_cancel(&turn_cancel);
                                }
                                steer_dispatch::Action::StartTurn => {
                                    start_turn(&cmd_tx, &mut cancel,
                                        UiCmd::Prompt(String::new(), Vec::new())).await;
                                    running = true;
                                    chat.begin_turn();
                                }
                                steer_dispatch::Action::Noop => {}
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
    // Last-resort guard against a tool/subagent ignoring the Quit cancel: bound
    // the worker wait so the terminal is restored instead of freezing on a blocked worker.
    let _ = tokio::time::timeout(Duration::from_secs(5), worker).await;
    Ok(session_id)
}

pub(crate) use crate::app_helpers::{
    apply_force_redraw, clear_pending_inputs, handle_mouse, initial_chat_view,
    mk_input_with_images, on_resize_event, poll_idle_resize, pre_key_intercept, push_user,
    snapshot_image_uris, start_turn, sys_tokens_for, worker_dead, MouseOutcome,
};
pub(crate) use crate::skill_display::{skill_token_display, skill_trigger};

#[cfg(test)]
#[path = "app_tests/mod.rs"]
mod tests;
