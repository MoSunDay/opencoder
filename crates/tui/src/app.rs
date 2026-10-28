use anyhow::Result;
use crossterm::event::Event;
use opencoder_core::Config;
use opencoder_llm::ChatStream;
use opencoder_session::SessionState;
use opencoder_store::Store;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cache_salt_menu::{handle_cache_salt_key, CacheSaltMenu, CacheSaltOutcome};
use crate::chat::ChatView;
use crate::command::CommandMenu;
use crate::key_handler::{handle_key, KeyAction};
use crate::menu::SkillMenu;
use crate::model_menu::ModelMenu;
use crate::queue_admitter;
use crate::render::{MouseHits, Term};
use crate::skill_persist::{act_plan_highlight, initial_skill_state};
use crate::task::{handle_task_key, TaskOutcome, TaskPicker};
use crate::terminal::consume_modifier_or_release;
use crate::worker::{UiCmd, UiEvent};
use crate::TuiOpts;
#[path = "app_bootstrap.rs"]
mod app_bootstrap;
#[path = "app_display.rs"]
mod app_display;
#[path = "app_loop.rs"]
pub(crate) mod app_loop;
#[path = "app_notepad.rs"]
mod app_notepad;
#[path = "app_submit.rs"]
mod app_submit;
#[path = "app_task.rs"]
mod app_task;
#[path = "steer_dispatch.rs"]
mod steer_dispatch;
#[path = "steer_fire.rs"]
mod steer_fire;
#[path = "subagent_input.rs"]
mod subagent_input;

pub async fn run(opts: &TuiOpts) -> Result<()> {
    app_bootstrap::run(opts).await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_app(
    opts: &TuiOpts,
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
    let mut cancel = CancellationToken::new();
    let session = session.with_cancel(cancel.clone());
    // These session handles are rebound together on task switches.
    let mut turn_cancel = session
        .turn_cancel
        .clone()
        .unwrap_or_else(|| Arc::new(std::sync::Mutex::new(CancellationToken::new())));
    let mut child_runtime = crate::worker::ChildRuntimeHandles::from_session(&session);
    let mut skill_handle = session.skill_prompt.clone();
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
    let mut queue_scroll: u32 = 0;
    let mut plan_edit: Option<crate::plan_edit::PlanEdit> = None;
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let mut bash_rx: Option<tokio::sync::oneshot::Receiver<String>> = None;
    let (initial_skill_body, mut sys_tokens, mut plan_skill_active) =
        initial_skill_state(&skill_handle, session.agent.name.as_str(), &workdir);
    // Cached system-prompt tokens for the subagent currently being viewed.
    // Computed once on entry (ctx-switch click) to avoid per-frame rebuild.
    let mut subagent_sys: u64 = 0;
    let mut queue_items =
        crate::queue_panel::restore_pending_mirrors(&store, &session_id, &mut chat.steer_items)
            .await;
    // Queue and steer admission run outside the render loop.
    let (admit_tx, mut admit_done_rx) =
        queue_admitter::spawn_admitter(Arc::clone(&store), config.network.proxy.clone());
    let mut admit_st = queue_admitter::AdmitUiState::default();
    let mut clear_confirm: Option<crate::clear_confirm::ClearConfirm> = None; // countdown guard
    let mut admitter_alive = true;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut task_picker: Option<TaskPicker> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut agent_menu: Option<crate::agent_menu::AgentMenu> = None;
    let mut model_menu: Option<ModelMenu> = None;
    let mut mcp_menu: Option<crate::mcp_menu::McpMenu> = None;
    let mut cli_menu: Option<crate::cli_menu::CliMenu> = None;
    let mut skill_toggle_menu: Option<crate::skill_menu::SkillMenu> = None;
    let mut ap_menu: Option<crate::ap_menu::ApMenu> = None;
    let mut cache_salt_menu: Option<CacheSaltMenu> = None;
    let mut keymap_menu: Option<crate::keymap_menu::KeymapMenu> = None;
    let mut keymap = crate::keymap::KeyBindings::from_config(&config);
    let (mut active_skill, mut active_skill_body) =
        crate::skill_display::skill_mirror_from_body(initial_skill_body);
    let mut anim_tick: u32 = 0;
    let mut mode_flash: Option<(String, u32)> = None;
    let mut last_esc: Option<Instant> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut shift_held = false;
    let mut copy_mode = false;
    let mut session_states: std::collections::HashMap<String, crate::session_ui::SessionUiState> =
        std::collections::HashMap::new();
    let (mut cmd_tx, cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(crate::worker::UI_EVENT_CAPACITY);

    let (mut sidecar_ask, worker) =
        crate::app_helpers::start_worker(session, evt_tx, cmd_rx, store.clone());

    let (mut input_rx, supervisor_active) = crate::app_helpers::start_input();
    let mut anim_ticker = tokio::time::interval(Duration::from_millis(app_loop::ANIM_TICK_MS));
    let mut frame_ms = config.tui_frame_ms();
    let mut frame_ticker = tokio::time::interval(Duration::from_millis(frame_ms));
    frame_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut body_ticker = tokio::time::interval(Duration::from_millis(app_loop::BODY_REFRESH_MS));
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
    let mut pending_task_pick = None;
    let mut selection: crate::remote::Selection = None;
    loop {
        let (changed, pick) =
            crate::remote::poll_ui(&mut agent_menu, &mut selection, &mut mode_flash, anim_tick);
        if changed {
            dirty = true;
            render_pending = true;
        }
        if pick.is_some() {
            pending_task_pick = pick;
        }
        if let Some(pick) = pending_task_pick.take() {
            if chat.remote {
                question_menu = None;
            } else {
                crate::question_menu::abandon_dialog(&mut question_menu, &question_hub);
            }

            let switched = app_task::switch_session(
                pick,
                opts,
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
                &mut sidecar_ask,
            )
            .await;
            match switched {
                Ok(()) => {
                    mode_flash = None;
                    pending_images.clear();
                    subagent_focus = None;
                    cancelled = false;
                    drain_pending = false;
                    clear_confirm = None;
                    plan_edit = None;
                    last_esc = None;
                    task_elapsed_ms = 0;
                    plan_skill_active = false;
                    admit_st = Default::default();
                    dirty = true;
                    render_pending = true;
                    body_refresh_pending = true;
                }
                Err(error) => mode_flash = Some((format!("{error:#}"), anim_tick)),
            }
        }
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
                    agent_menu.as_ref(),
                    model_menu.as_ref(),
                    mcp_menu.as_ref(),
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
                    app_loop::body_is_top_level(&chat, subagent_focus),
                    config.autopilot.mode,
                    &display_mode,
                    plan_skill_active,
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
                // 输入即帧：Key/Paste/Mouse/Resize 立即置 render_pending，
                // 按键回显不再等待 fps 帧周期的下一个 tick。
                dirty = true;
                if app_loop::input_event_prompts_frame(&ev) {
                    render_pending = true;
                }
                match ev {
                    Event::Key(k) => {
                        // Armed clear-context guard: Enter or a second Shift+Tab fires, Esc 回撤, rest inert.
                        if clear_confirm.is_some() {
                            if app_loop::handle_confirm_key(&mut clear_confirm, k, &mut input, &mut cursor_idx, &mut undo_state, &mut chat, &cmd_tx, &mut cancel, &mut running, &mut follow, &mut sys_tokens, &mut mode_flash, anim_tick, &workdir, &admit_tx, &mut admit_st, &mut queue_items, &mut pending_images, &session_id, &mut history, &mut hist_idx, &mut child_runtime, &mut cancelled).await { break; }
                            dirty = true;
                            continue;
                        }
                        if consume_modifier_or_release(&k, &mut shift_held, copy_mode) {
                            dirty = true;
                            continue;
                        }
                        if crate::copy_mode::handle_key(&k, &mut copy_mode, &keymap, &mut scroll, &mut follow) { dirty = true; render_pending = true; continue; }
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
                                    selection = None; pending_task_pick = Some(pick);
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
                                &sidecar_ask, &mut running, &mut follow, &store,
                                &session_id, &mut task_picker, &mut model_menu, &mut mcp_menu, &mut cli_menu, &mut skill_toggle_menu, &mut ap_menu,
                                &mut cache_salt_menu, &mut keymap_menu, &agent_name,
                                &mut input, &mut cursor_idx,
                                &mut config, &workdir,
                                &mut mode_flash, anim_tick, &mut sys_tokens,
                                &mut plan_edit,
                                &mut notepad,
                                &mut clear_confirm,
                                &mut agent_menu,
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
                            &sidecar_ask,
                        ) {
                            apply_force_redraw(
                                needs_clear,
                                &mut *terminal,
                                &mut render_pending,
                                &mut skip_next_render,
                            );
                            continue;
                        }
                        let action = handle_key(
                            k,
                            &keymap,
                            &mut input,
                            &mut cursor_idx,
                            &history,
                            &mut hist_idx,
                            running, chat.subagents_running > 0,
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
                            chat.sidecar_focus,
                            input_disabled,
                            &mut undo_state,
                            &mut queue_scroll,
                            &mut agent_menu,
                        );
                        if chat.remote && crate::remote::local_control(&action) {
                            mode_flash = Some(("This control is available for /agent self tasks".into(), anim_tick));
                            continue;
                        }
                        match action {
                            KeyAction::Submit(text) => {
                                if app_submit::handle_submit_action(
                                    text, &mut running, &admit_tx, &mut admit_st,
                                    &mut queue_items, &mut pending_images, &session_id,
                                    &mut history, &mut hist_idx, &mut active_skill,
                                    &mut active_skill_body, &mut sys_tokens, &agent_name,
                                    &workdir, &skill_handle, &mut chat, &sidecar_ask, &store,
                                    &mut plan_skill_active, &mut clear_confirm, &mut mode_flash,
                                    anim_tick, &mut plan_edit, &mut notepad, &mut task_picker,
                                    &mut model_menu, &mut mcp_menu,
                                    &mut cli_menu, &mut skill_toggle_menu, &mut ap_menu,
                                    &mut cache_salt_menu, &mut config, &cmd_tx, &mut cancel,
                                    &mut task_elapsed_ms, &mut cancelled, &mut follow,
                                    &mut body_refresh_pending, &mut agent_menu,
                                )
                                .await
                                    == app_loop::LoopFlow::Quit
                                {
                                    break;
                                }
                            }
                            KeyAction::SubagentSteer(text) => {
                                subagent_input::handle_subagent_steer(&store, &child_runtime.steer_gates, &mut chat, subagent_focus, text, &mut pending_images, &mut input, &mut cursor_idx).await;
                                follow = true;
                            }
                            KeyAction::Steer(text) => {
                                // Typed clear_context while running: arm the countdown
                                // guard instead of admitting the steer — firing queues
                                // it for the idle boundary; Esc 回撤 restores the draft.
                                if crate::clear_confirm::maybe_arm(&mut clear_confirm, &mut chat, &mut mode_flash, anim_tick, &text, Some(text.clone())) {
                                    dirty = true;
                                    continue;
                                }
                                // Deferred steer: the raw text (tokens included) is admitted
                                // verbatim; the runner absorbs it at the turn boundary via
                                // record_compound, which resolves/activates/persists the
                                // skill THEN — a `$skill` steer must not arm mid-turn.
                                // No plan arm either: consumption-time only (TurnDone(plan)
                                // reads the persisted plan-phase counter).
                                let raw = text.trim().to_string();
                                if !raw.is_empty() {
                                    // Off-loop actor owns the store write; a failed
                                    // hand-off flashes (↑ recovers). `>` = interrupt.
                                    steer_submit_flash(
                                        &admit_tx, &mut admit_st, &mut chat.steer_items,
                                        &mut pending_images, &session_id, &raw, anim_tick,
                                        &mut mode_flash,
                                    );
                                }
                                push_history(&mut history, &mut hist_idx, &text);
                                // Enter admits without interrupting (`>` interrupts instead).
                                follow = true;
                            }
                            KeyAction::Queue(text) => {
                                // Tab-queue: raw-text deferred admission — skill resolution
                                // happens at consumption (idle boundary, record_compound).
                                // The off-loop actor owns the store write; this loop never
                                // waits on db_lock; a failed hand-off flashes (text via ↑).
                                queue_submit_flash(
                                    &text, &admit_tx, &mut admit_st, &mut queue_items,
                                    &mut pending_images, &session_id, anim_tick, &mut mode_flash,
                                );
                                push_history(&mut history, &mut hist_idx, &text);
                                follow = true;
                            }
                            KeyAction::QueueUnsupported => {
                                mode_flash = Some(queue_unsupported_flash(anim_tick));
                            }
                            KeyAction::ModeSwitchBlocked => {
                                mode_flash = Some(mode_switch_busy_flash(anim_tick));
                            }
                            KeyAction::OpenTask => {
                                selection = None;
                                let sessions = store.list_sessions(&opencoder_store::SessionFilter::default()).await.unwrap_or_default();
                                task_picker = Some(TaskPicker::new(sessions,session_id.clone()));
                            }
                            KeyAction::SelectAgent(name) => {
                                match name.as_str() {
                                    "" => { selection = None; agent_menu = Some(crate::agent_menu::AgentMenu::configured(config.clone())); }
                                    "self" => { selection = None; pending_task_pick = Some(crate::task::TaskPick::New); }
                                    _ => { selection = Some(crate::remote::request(config.clone(),name)); mode_flash = Some(("Connecting to Server…".into(),anim_tick)); }
                                }
                            }
                            KeyAction::SidecarAsk(question) => {
                                if question.is_empty() {
                                    crate::sidecar_ui::enter_panel(&mut chat, &sidecar_ask);
                                    follow = true;
                                } else if !chat.sidecar_focus {
                                    crate::sidecar_ui::enter_panel(&mut chat, &sidecar_ask);
                                    match sidecar_ask
                                        .try_send(crate::sidecar_ui::SidecarCmd::Ask(question.clone()))
                                    {
                                        Ok(()) => {
                                            // Instant echo: the actor needs a beat to
                                            // build its conv before SidecarStart; the
                                            // Busy path must NOT echo.
                                            crate::sidecar_ui::echo_question(&mut chat, &question);
                                            follow = true;
                                        }
                                        Err(_) => {
                                            mode_flash = Some((
                                                crate::sidecar_ui::SIDECAR_BUSY_FLASH.to_string(),
                                                anim_tick,
                                            ));
                                        }
                                    }
                                } else {
                                    // Follow-up inside the focused panel: the SAME
                                    // conversation continues (no reset — Q/A
                                    // continuity). Fire-and-forget into the actor:
                                    // never blocks the UI loop, never touches
                                    // steer/queue.
                                    match sidecar_ask
                                        .try_send(crate::sidecar_ui::SidecarCmd::Ask(question.clone()))
                                    {
                                        Ok(()) => {
                                            crate::sidecar_ui::echo_question(&mut chat, &question);
                                            chat.sidecar_focus = true;
                                            follow = true;
                                        }
                                        Err(_) => {
                                            mode_flash = Some((
                                                crate::sidecar_ui::SIDECAR_BUSY_FLASH.to_string(),
                                                anim_tick,
                                            ));
                                        }
                                    }
                                }
                            }
                            KeyAction::SwitchAgent(name) => {
                                let mode = app_loop::ModeSwitch::for_agent(&name);
                                if app_loop::dispatch_mode_switch(
                                    mode, &cmd_tx, &mut cancel, &mut running, &mut follow, &mut chat,
                                    &mut sys_tokens, &mut mode_flash, anim_tick, &workdir,
                                ).await == app_loop::LoopFlow::Quit { break; }
                            }
                            KeyAction::SetSkill(opt) => {
                                crate::skill_persist::apply_skill_selection(
                                    &opt, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir,
                                    &skill_handle, &store, &session_id,
                                )
                                .await;
                                plan_skill_active = act_plan_highlight(active_skill.as_deref());
                            }
                            KeyAction::ArmClearConfirm { rest, draft } => {
                                crate::clear_confirm::engage(&mut clear_confirm, &mut chat, &mut mode_flash, anim_tick, rest, draft);
                                dirty = true;
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
                                app_loop::paste_clipboard_image(&mut chat, &mut pending_images).await;
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
                            store.as_ref(), &mut queue_scroll, &mut pending_images,
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
                        // Modal-priority paste routing (mirrors Event::Key); empty pastes try a silent clipboard-image read. (clippy's collapsible_match suggestion would put an `.await` in a match guard, which Rust forbids.)
                        #[allow(clippy::collapsible_match)]
                        if app_loop::handle_paste_event(&pasted, &mut plan_edit, &mut notepad, task_picker.is_some(), cache_salt_menu.is_some(), keymap_menu.is_some(), skill_toggle_menu.is_some(), &mut model_menu, &mut mcp_menu, &mut cli_menu, &mut command_menu, &mut question_menu, &mut input, &mut cursor_idx, &mut pending_images, &mut img_asm, &mut chat, &workdir).await { continue; }
                    }
                    _ => {}
                }
            }
            maybe_done = admit_done_rx.recv(), if admitter_alive => {
                match maybe_done {
                    Some(done) => {
                        let o = crate::idle_rekick::on_admit_done(done, &mut admit_st, &mut queue_items, &mut chat.steer_items, &mut pending_images, running, &store, &session_id, &cmd_tx, &mut cancel).await;
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
                    maybe_ev, &mut chat, &store, &session_id, &mut queue_items,
                    &mut plan_skill_active, &mut admit_st, &mut running,
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
                                &mut sys_tokens, &agent_name, &workdir, &mut plan_skill_active,
                            );
                            body_refresh_pending = true; // idle: land the final frame ([tok cost] total) behind the 333ms body ticker
                        }
                        dirty = true;
                    }
                    app_loop::LoopFlow::Redraw => continue,
                }
            }
            _ = anim_ticker.tick() => {
                if running { anim_tick = anim_tick.wrapping_add(1); dirty = true; }
                if clear_confirm.is_some() {
                    anim_tick = anim_tick.wrapping_add(1);
                    if app_loop::confirm_tick(&mut clear_confirm, &mut mode_flash, anim_tick, &cmd_tx, &mut cancel, &mut running, &mut follow, &mut chat, &mut sys_tokens, &workdir, &admit_tx, &mut admit_st, &mut queue_items, &mut pending_images, &session_id, &mut history, &mut hist_idx).await { break; }
                    dirty = true;
                }
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
    // Quit-path terminal quiesce: stop the terminal from reporting further
    // input (Kitty pop + mouse/paste off) and absorb the release/repeat
    // reports of the quitting keypress before they can strand in the tty
    // queue and be echoed as `442;1:3u`-style garbage by the shell (no tmux).
    // Must run BEFORE `finish` while raw mode is still on, so drained bytes
    // are never echoed by the tty line discipline.
    crate::input::drain_shutdown(&mut input_rx).await;
    app_bootstrap::finish(&supervisor_active, cmd_tx, worker).await;
    Ok(session_id)
}
pub(crate) use crate::app_helpers::{
    apply_force_redraw, handle_mouse, initial_chat_view, mode_switch_busy_flash, on_resize_event,
    poll_idle_resize, pre_key_intercept, push_history, queue_submit_flash, queue_unsupported_flash,
    steer_submit_flash, worker_dead, MouseOutcome,
};
#[cfg(test)]
#[path = "app_tests/mod.rs"]
mod tests;
