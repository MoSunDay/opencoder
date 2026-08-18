use anyhow::Result;
use crossterm::event::Event;
use opencoder_core::Config;
use opencoder_llm::{estimate, ChatStream};
use opencoder_session::SessionState;
use opencoder_store::Store;
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Arc};
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
use crate::queue_admitter;
use crate::render::{MouseHits, Term};
use crate::skill_persist::resolve_persist;
use crate::task::{handle_task_key, TaskOutcome, TaskPicker};
use crate::terminal::consume_modifier_or_release;
use crate::worker::{process_cmd, UiCmd, UiEvent};
use crate::TuiOpts;
#[path = "app_bootstrap.rs"]
mod app_bootstrap;
#[path = "app_display.rs"]
mod app_display;
#[path = "app_loop.rs"]
pub(crate) mod app_loop;
#[path = "app_notepad.rs"]
mod app_notepad;
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
    // Question hub: the TUI is the interactive question listener, attached
    // before the worker spawns so the first turn may already ask the user.
    let mut question_hub = session.question_hub.clone();
    question_hub.attach();
    let mut question_menu = crate::question_menu::dialog_state();
    let mut chat = initial_chat_view(&session, &store).await;
    chat.annotation_text = session.requirement.clone();
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
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let mut bash_rx: Option<tokio::sync::oneshot::Receiver<String>> = None;
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
    // Off-loop queue admission: Tab / Enter-while-running submits go through a
    // dedicated actor so this event loop never waits on the store-wide db_lock
    // (held in bursts by the running turn's message/subagent flushers). The
    // optimistic temp row appears instantly; reconciliation arrives on
    // `admit_done_rx` (select branch below).
    let (admit_tx, mut admit_done_rx) = queue_admitter::spawn_admitter(Arc::clone(&store));
    let mut admit_st = queue_admitter::AdmitUiState::default();
    let mut admitter_alive = true;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut task_picker: Option<TaskPicker> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut model_menu: Option<ModelMenu> = None;
    let mut mcp_menu: Option<crate::mcp_menu::McpMenu> = None;
    let mut envs_menu: Option<crate::envs_menu::EnvsMenu> = None;
    let mut cli_menu: Option<crate::cli_menu::CliMenu> = None;
    let mut skill_toggle_menu: Option<crate::skill_menu::SkillMenu> = None;
    let mut ap_menu: Option<crate::ap_menu::ApMenu> = None;
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
    let mut copy_mode = false;
    let mut session_states: std::collections::HashMap<String, crate::session_ui::SessionUiState> =
        std::collections::HashMap::new();
    let (mut cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(crate::worker::UI_EVENT_CAPACITY);

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
                    mcp_menu.as_ref(),
                    envs_menu.as_ref(),
                    cli_menu.as_ref(),
                    skill_toggle_menu.as_ref(),
                    ap_menu.as_ref(),
                    cache_salt_menu.as_ref(),
                    keymap_menu.as_ref(),
                    question_menu.as_ref(),
                    &mut hits,
                    &mut viewport,
                    shift_held,
                    copy_mode,
                    &pending_images,
                    input_disabled,
                    tail_ms,
                    task_elapsed_ms,
                    subagent_focus.is_none(),
                    config.autopilot.mode,
                    &display_mode,
                    notepad.as_ref(),
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
                        if consume_modifier_or_release(&k, &mut shift_held, copy_mode) {
                            dirty = true;
                            continue;
                        }
                        if crate::copy_mode::handle_key(&k, &mut copy_mode, &keymap) { dirty = true; render_pending = true; continue; }
                        if plan_edit.is_some() {
                            let f = app_loop::dispatch_plan_edit_key(&mut plan_edit, k, &mut chat, &cmd_tx, terminal).await;
                            if f == app_loop::LoopFlow::Quit { break; } continue;
                        }
                        let r = app_notepad::key(&mut notepad, k).await;
                        if r.handled {
                            dirty = true; continue;
                        }
                        // Task picker modal: intercept all keys while open.
                        if task_picker.is_some() {
                            match handle_task_key(&mut task_picker, k) {
                                TaskOutcome::Pick(pick) => {
                                    // Drop any pending question dialog: abandoning its hub
                                    // entry unblocks the tool with the skip reply.
                                    crate::question_menu::abandon_dialog(&mut question_menu, &question_hub);

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
                                        &mut question_hub,
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
                        if mcp_menu.is_some() {
                            let _ = app_loop::handle_mcp_outcome(
                                &mut mcp_menu, k, &mut config, &cmd_tx, &mut chat, &workdir,
                            ).await;
                            continue;
                        }
                        if envs_menu.is_some() {
                            match app_loop::handle_envs_outcome(&mut envs_menu, k, &mut client, &mut config, &mut model_label, &mut compaction_threshold, &mut context_limit, &mut frame_ms, &mut frame_ticker, &cmd_tx, &mut chat, &workdir).await {
                                app_loop::LoopFlow::Quit => break, app_loop::LoopFlow::Redraw => continue, _ => {}
                            }
                            continue;
                        }
                        if cli_menu.is_some() {
                            let _ = app_loop::handle_cli_outcome(
                                &mut cli_menu, k, &mut config, &cmd_tx, &mut chat, &workdir,
                            ).await;
                            continue;
                        }
                        // `/skill` toggle modal (after cli/mcp blocks, which `continue` first).
                        if skill_toggle_menu.is_some() {
                            let _ = app_loop::handle_skill_outcome(&mut skill_toggle_menu, k, &mut config, &cmd_tx, &mut chat, &workdir).await; continue;
                        }
                        // `/ap` mode-picker modal (same slot.take() pattern as /skill).
                        if ap_menu.is_some() {
                            let _ = app_loop::handle_ap_outcome(&mut ap_menu, k, &mut config, &cmd_tx, &mut chat, &workdir).await; continue; }
                        // Question dialog: answers resolve on the hub, mid-turn.
                        if question_menu.is_some() {
                            // Wrap width mirrors the popup renderer so Up/Down
                            // cursor movement tracks the drawn wrapped rows.
                            let q_width = terminal
                                .size()
                                .map(|r| crate::question_menu::input_wrap_width(r.width))
                                .unwrap_or(55);
                            crate::question_menu::route_question_key(
                                &mut question_menu, k, &question_hub, q_width,
                            );
                            dirty = true;
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
                                &session_id, &mut task_picker, &mut model_menu, &mut mcp_menu, &mut envs_menu, &mut cli_menu, &mut skill_toggle_menu, &mut ap_menu,
                                &mut cache_salt_menu, &mut keymap_menu, &agent_name,
                                &mut input, &mut cursor_idx,
                                &mut config, &workdir,
                                &mut mode_flash, anim_tick, &mut sys_tokens,
                                &mut plan_edit,
                                &mut notepad,
                            )
                            .await
                            {
                                app_loop::LoopFlow::Quit => break,
                                app_loop::LoopFlow::Proceed => {}
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
                                if running {
                                    // Submit while running is reachable only via BackTab's
                                    // compound `/plan …` (Enter/Tab map to Steer/Queue when
                                    // running), so no bare slash command can land here.
                                    // Deferred: the raw text (tokens included) queues verbatim;
                                    // the runner's record_compound resolves/activates/
                                    // persists the skill at the idle boundary — never now,
                                    // or it would fire inside the running turn.
                                    queue_admitter::handle_queue(
                                        &text, &admit_tx, &mut admit_st, &mut queue_items,
                                        &mut pending_images, &session_id,
                                    );
                                    push_history(&mut history, &mut hist_idx, &text);
                                    continue;
                                }
                                // Idle submit: the turn starts now, so eager skill
                                // activation (and persistence) is the correct timing.
                                let (clean, _unresolved) = resolve_persist(
                                    &text, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir, &skill_handle, &mut chat,
                                    &store, &session_id,
                                ).await;
                                let clean = clean.trim().to_string();
                                let clean = crate::control_helpers::forward_skill_if_compound(&text, &clean);
                                // NOTE: no compound `/plan` arm here — arming is
                                // consumption-time (TurnDone(plan) reads the persisted
                                // plan-phase counter).
                                // Intercept /annotation: open the editor instead of submitting
                                if let Some(action) = crate::command::parse(&clean) {
                                    // Unified slash-command dispatch: route recognized `/cmd`
                                    // through the same handler as the `/` popup picker.
                                    let f = app_loop::dispatch_slash_action(
                                        action, &cmd_tx, &mut cancel, &mut chat,
                                        &mut running, &mut follow, &store,
                                        &session_id, &mut task_picker, &mut model_menu, &mut mcp_menu, &mut envs_menu, &mut cli_menu, &mut skill_toggle_menu,
                                        &mut ap_menu, &mut cache_salt_menu, &agent_name, &mut input, &mut cursor_idx,
                                        &mut config, &workdir,
                                        &mut mode_flash, anim_tick, &mut sys_tokens,
                                        &mut plan_edit, &mut notepad,
                                    )
                                    .await;
                                    match f {
                                        app_loop::LoopFlow::Quit => break,
                                        _ => push_history(&mut history, &mut hist_idx, &text),
                                    }
                                } else if clean.is_empty() {
                                    if active_skill.is_some() {
                                        if !text.is_empty() {
                                            push_user(&mut chat, &mut history, &mut hist_idx, &text);
                                        }
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
                                        body_refresh_pending = true;
                                    }
                                } else {
                                    push_user(&mut chat, &mut history, &mut hist_idx, &text);
                                    chat.context_used += estimate(&clean) as u64;
                                    let image_uris = snapshot_image_uris(&pending_images);
                                    if !start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt(clean, image_uris)).await
                                    {
                                        worker_dead(&mut chat);
                                        break;
                                    }
                                    pending_images.clear();
                                    task_elapsed_ms = 0;
                                    cancelled = false;
                                    running = true;
                                    follow = true;
                                    chat.note_requirement_submitted();
                                    chat.begin_turn();
                                    body_refresh_pending = true;
                                }
                            }
                            KeyAction::SubagentSteer(text) => {
                                subagent_input::handle_subagent_steer(&store, &child_runtime.steer_gates, &mut chat, subagent_focus, text, &mut pending_images, &mut input, &mut cursor_idx).await;
                                follow = true;
                            }
                            KeyAction::Steer(text) => {
                                // Deferred steer: the raw text (tokens included) is admitted
                                // verbatim; the runner absorbs it at the turn boundary via
                                // record_compound, which resolves/activates/persists the
                                // skill THEN — a `$skill` steer must not arm mid-turn.
                                // No plan arm either: consumption-time only (TurnDone(plan)
                                // reads the persisted plan-phase counter).
                                let raw = text.trim().to_string();
                                if !raw.is_empty() {
                                    let seq = steer_fire::admit_keyboard_steer(
                                        &store, &session_id, &raw, &raw,
                                        &mut pending_images, &mut chat,
                                    )
                                    .await;
                                    // Store failure must not vanish silently; ↑ history still holds the text.
                                    if let Some(flash) = steer_fire::flash_on_admit_failure(seq) {
                                        mode_flash = Some((flash.to_string(), anim_tick));
                                    }
                                }
                                push_history(&mut history, &mut hist_idx, &text);
                                // Enter admits without interrupting (`>` interrupts instead).
                                follow = true;
                            }
                            KeyAction::Queue(text) => {
                                // Tab-queue: raw-text deferred admission — skill resolution
                                // happens at consumption (idle boundary, record_compound).
                                // The off-loop actor owns the store write; this loop never
                                // waits on db_lock.
                                queue_admitter::handle_queue(
                                    &text, &admit_tx, &mut admit_st, &mut queue_items,
                                    &mut pending_images, &session_id,
                                );
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
                                crate::skill_persist::apply_skill_selection(
                                    &opt, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir,
                                    &skill_handle, &store, &session_id,
                                )
                                .await;
                            }
                            KeyAction::Cancel => {
                                app_loop::cancel_running_turn(
                                    &mut chat, &mut cancel,
                                    &mut child_runtime, &mut running, &mut cancelled, &mut follow,
                                ).await;
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
                            KeyAction::Bash(cmd) => { app_notepad::handle_bash(&cmd, &mut chat, &mut bash_rx, &workdir, &mut history, &mut hist_idx); dirty = true; }
                            KeyAction::None => {}
                        }
                    }
                    Event::Mouse(m) => {
                        if crate::copy_mode::is_active(copy_mode, shift_held) { dirty = true; continue; }
                        if keymap_menu.is_some() { if let app_loop::LoopFlow::Quit = app_loop::handle_keymap_mouse_event(&mut keymap_menu, &hits.keymap_btns, &m, &mut config, &mut keymap, &workdir, &cmd_tx).await { break }
                            dirty = true; render_pending = true; continue; }
                        let outcome = handle_mouse(
                            m, &hits, &mut scroll, &mut follow, &mut chat,
                            &mut subagent_focus,
                            &mut subagent_sys, &workdir, &mut queue_items, &session_id,
                            store.as_ref(), &mut queue_scroll,
                        )
                        .await;
                        if outcome == MouseOutcome::SteerSubmit {
                            app_loop::steer_submit_after_mouse(
                                &cmd_tx, &mut cancel, subagent_focus, &mut running,
                                &mut chat, &mut follow, &child_runtime, &turn_cancel,
                            ).await;
                        }
                        dirty = true;
                    }
                    Event::Resize(_, _) => on_resize_event(terminal, &mut last_size)?,
                    Event::Paste(pasted) => {
                        // Modal-priority paste routing (mirrors Event::Key);
                        // empty pastes try a silent clipboard-image read.
                        // (clippy's collapsible_match suggestion would put an
                        // `.await` in a match guard, which Rust forbids.)
                        #[allow(clippy::collapsible_match)]
                        if app_loop::handle_paste_event(
                            &pasted, task_picker.is_some(), cache_salt_menu.is_some(), keymap_menu.is_some(),
                            skill_toggle_menu.is_some(),
                            &mut model_menu, &mut mcp_menu, &mut envs_menu, &mut cli_menu, &mut command_menu, &mut question_menu,
                            &mut input, &mut cursor_idx, &mut pending_images, &mut img_asm,
                            &mut chat, &workdir,
                        ).await { continue; }
                    }
                    _ => {}
                }
            }
            maybe_done = admit_done_rx.recv(), if admitter_alive => {
                match maybe_done {
                    Some(done) => {
                        let o = crate::idle_rekick::on_admit_done(done, &mut admit_st, &mut queue_items, &mut pending_images, running, &store, &session_id, &cmd_tx, &mut cancel).await;
                        if let Some(flash) = o.flash { mode_flash = Some((flash.to_string(), anim_tick)); }
                        match o.flow {
                            crate::idle_rekick::AdmitDoneFlow::Started => { running = true; follow = true; cancelled = false; chat.begin_turn(); }
                            crate::idle_rekick::AdmitDoneFlow::WorkerDead => { worker_dead(&mut chat); break; }
                            _ => {}
                        }
                        dirty = true;
                    }
                    // Actor gone (only after a panic): stop polling the closed
                    // channel — a permanently-ready None would busy-spin.
                    None => admitter_alive = false,
                }
            }
            maybe_ev = evt_rx.recv() => {
                let np_flow = app_loop::fold_ui_events(
                    maybe_ev, &mut chat, &store, &session_id, &mut queue_items, &mut admit_st, &mut running,
                    &mut cancelled, &mut drain_pending, &mut skip_next_render, &mut follow,
                    &cmd_tx, &mut cancel, &mut evt_rx, &mut notepad,
                    &mut question_menu, &question_hub,
                )
                .await;
                match np_flow {
                    app_loop::LoopFlow::Quit => break,
                    app_loop::LoopFlow::Proceed => {
                        // Consumption-time skill activation (runner record_compound
                        // at the idle boundary) rewrote the shared handle; mirror
                        // it so sys_tokens and the /task snapshots stay truthful.
                        // Only when idle: mid-run the turn owns the handle and the
                        // next TurnDone re-syncs.
                        if !running {
                            crate::app_helpers::refresh_skill_mirrors(
                                &skill_handle, &mut active_skill, &mut active_skill_body,
                                &mut sys_tokens, &agent_name, &workdir,
                            );
                        }
                        dirty = true;
                    }
                    app_loop::LoopFlow::Redraw => continue,
                }
            }
            _ = anim_ticker.tick() => {
                if running { anim_tick = anim_tick.wrapping_add(1); dirty = true; }
                if app_notepad::poll_bash(&mut bash_rx, &mut chat) { dirty = true; }
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
    app_bootstrap::finish(&supervisor_active, cmd_tx, worker).await;
    Ok(session_id)
}
pub(crate) use crate::app_helpers::{
    apply_force_redraw, handle_mouse, initial_chat_view, on_resize_event, poll_idle_resize,
    pre_key_intercept, push_history, push_user, snapshot_image_uris, start_turn, sys_tokens_for,
    worker_dead, MouseOutcome,
};
pub(crate) use crate::skill_display::skill_trigger;
#[cfg(test)]
#[path = "app_tests/mod.rs"]
mod tests;
