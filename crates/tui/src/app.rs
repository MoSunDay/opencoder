use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::Event;
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{estimate, ChatClient, ChatStream};
use opencoder_session::SessionState;
use opencoder_store::{Delivery, LibsqlStore, Store};
use ratatui::backend::CrosstermBackend;
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
use crate::render::{render, MouseHits, Term};
use crate::task::{handle_task_key, TaskOutcome, TaskPicker};
use crate::terminal::TerminalGuard;
use crate::worker::{process_cmd, UiCmd, UiEvent};
use crate::TuiOpts;

#[path = "app_loop.rs"]
mod app_loop;

#[path = "app_task.rs"]
mod app_task;

/// Animation tick rate for the running spinner (10 FPS).
const ANIM_TICK_MS: u64 = 100;
/// How long the plan/act switch flash stays visible, in anim ticks (~100ms each).
const MODE_FLASH_TICKS: u32 = 15;
/// Body (info area) refresh interval -- the cached ChatView snapshot is rebuilt
/// at this cadence (3 FPS), decoupling text layout from the fast spinner.
const BODY_REFRESH_MS: u64 = 333;

/// Whether a transient flash started at `start` is still visible at `now`,
/// given a lifetime of `ticks` anim ticks. Uses wrapping subtraction so it
/// stays correct across the u32 wraparound of `anim_tick`.
pub(crate) fn flash_visible(start: u32, now: u32, ticks: u32) -> bool {
    now.wrapping_sub(start) < ticks
}

/// Copy-paste-ready command to resume a session by id.
pub(crate) fn resume_hint(id: &str) -> String {
    format!("resume with: opencoder -s {id}")
}

/// Resolve the `(base_url, api_key)` pair used to build the LLM client at TUI
/// startup. Selects the provider whose name matches the `model`'s `provider/`
/// prefix via `Config::resolve_endpoint`, so a `model` like
/// `deepseek/deepseek-chat` resolves against `providers["deepseek"]` rather
/// than the legacy top-level `provider.base_url`. Extracted as a testable seam
/// for the startup path, which otherwise only runs inside `run`.
pub(crate) fn startup_endpoint(config: &Config) -> Result<opencoder_core::Endpoint> {
    Ok(config.resolve_endpoint()?)
}

pub async fn run(opts: &TuiOpts) -> Result<()> {
    let workdir = opts
        .workdir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let config = Config::load(&workdir)?;
    let ep = startup_endpoint(&config)?;
    let client: Arc<dyn ChatStream> = Arc::new(ChatClient::new(
        &ep.base_url,
        &ep.api_key,
        &ep.headers,
        config.network.proxy.as_deref(),
    )?);

    let store: Arc<dyn Store> = {
        let data_dir = data_dir_for(&workdir);
        tokio::fs::create_dir_all(&data_dir).await.ok();
        Arc::new(LibsqlStore::open(data_dir.join("opencoder.db")).await?)
    };

    // Resume an existing session if --session was given, otherwise start fresh.
    let replay_cancel = CancellationToken::new();
    let session = if let Some(id) = &opts.session {
        // Try as a session ID first; if not found, try as a subagent
        // task_id to resolve the parent session.
        let effective_id = if store.get_session(id).await?.is_none() {
            if let Some(task) = store.get_subagent_task(id).await? {
                task.parent_session_id
            } else {
                id.clone()
            }
        } else {
            id.clone()
        };
        opencoder_session::resume::resume_and_replay(
            store.clone(),
            &effective_id,
            config.clone(),
            client.clone(),
            workdir.clone(),
            Some(replay_cancel.clone()),
        )
        .await?
    } else {
        let agent_name = config.agent.default.clone();
        let agent = resolve_agent(&agent_name)
            .or_else(|| resolve_agent("act"))
            .context("agent")?;
        SessionState::new(
            opencoder_session::runner::new_id(),
            agent,
            config.clone(),
            client.clone(),
            workdir.clone(),
        )
        .with_store(store.clone())
    };

    let session_id = session.id.clone();
    let context_limit = session.config.context_limit();
    let model_label = session.model.clone();

    // Terminal enter/restore is RAII: `TerminalGuard`'s Drop — and the panic
    // hook it installs — restore raw/alt-screen/mouse/kitty state on ANY exit
    // path (normal return, `?` error, or a panic that unwinds). This removes
    // the old "cleanup only ran on the happy path" trap that bricked the
    // terminal on any panic, leaving the user with a frozen last frame, no
    // echo, and ineffective Ctrl+C/D.
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Term::new(backend)?;

    let final_id = run_app(
        &mut terminal,
        session,
        store,
        session_id,
        context_limit,
        model_label,
        workdir,
        config,
        client,
    )
    .await?;

    // Restore the real terminal *before* printing so the hint lands on the
    // actual screen instead of being swallowed by the alt-screen buffer.
    drop(_guard);
    eprintln!("\n\x1b[2m{}\x1b[0m", resume_hint(&final_id));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_app(
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
    let mut skill_handle = session.skill_prompt.clone();

    let mut chat = if !session.messages.is_empty() {
        // A resumed session carries persisted history: rebuild the chat view
        // from it so the transcript is visible on startup (mirroring /task
        // switch-back and TranscriptReset), instead of a blank view.
        crate::session_ui::replay_into_chat(
            &session.agent.name,
            &session.messages,
            &store,
            &session.id,
        )
        .await
    } else {
        crate::chat::ChatView {
            agent: session.agent.name.clone(),
            ..Default::default()
        }
    };
    let mut input = String::new();
    let mut cursor_idx: usize = 0;
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut running = false;
    let mut pending_handoff: Option<String> = None;
    let mut run_elapsed_ms: u64 = 0;
    let mut last_clock = Instant::now();
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut show_help = false;
    let mut scroll: u16 = 0;
    let mut follow = true;
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
    let mut parent_scroll: u16 = 0;
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
    // Unlike `crossterm::EventStream`, whose reader could wedge forever on a
    // half-disambiguated Esc sequence under the Kitty protocol (freezing the
    // whole loop, Ctrl+C/D included), the bounded poll wakes at least every
    // 150ms, so this loop can never be starved of input.
    let (mut input_rx, _input_handle) = spawn_input_pump();
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
    // Persisted across loop iterations: always equals the LAST rendered
    // layout (== what is on screen). The event loop forwards `&hits` to
    // `handle_mouse` on the SAME iteration a click arrives, and a click
    // sets `dirty=true` so `hits` refreshes next frame. Declaring this
    // INSIDE the loop resets it to `MouseHits::default()` every turn; when
    // no render runs (idle state, `dirty=false`) the rects are empty and
    // EVERY arrow click is silently dropped. Keep this OUTSIDE `loop {}`.
    let mut hits = MouseHits::default();

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
            &model_label,
            &workdir,
        );
        // Refresh the body cache at BODY_REFRESH_MS cadence (3 FPS). Between
        // refreshes the spinner still animates at full frame rate because it is
        // driven by the real-time anim_tick, not the cached blocks.
        if dirty && (body_refresh_pending || display_chat_cached.is_none()) {
            display_chat_cached = Some(display_chat.clone());
            body_refresh_pending = false;
        }
        let render_chat = display_chat_cached.as_ref().unwrap_or(display_chat);
        if dirty && render_pending {
            if !skip_next_render {
                render(
                    terminal,
                    render_chat,
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
                    &chat.steer_items,
                    &queue_items,
                    &mut scroll,
                    follow,
                    anim_tick,
                    mode_flash.as_ref().and_then(|(t, s)| {
                        if pending_handoff.is_some() {
                            Some("\u{2192} act (pending)")
                        } else if flash_visible(*s, anim_tick, MODE_FLASH_TICKS) {
                            Some(t.as_str())
                        } else {
                            None
                        }
                    }),
                    skill_menu.as_ref(),
                    task_picker.as_ref(),
                    command_menu.as_ref(),
                    model_menu.as_ref(),
                    cache_salt_menu.as_ref(),
                    &mut hits,
                    selection,
                    copy_status.as_ref().and_then(|(msg, t)| {
                        if t.elapsed() < Duration::from_secs(2) {
                            Some(msg.as_str())
                        } else {
                            None
                        }
                    }),
                    subagent_focus.is_some(),
                    run_elapsed_ms,
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
                                &mut running, &mut follow, &store, &session_id, &mut task_picker,
                                &mut model_menu, &config, &mut cache_salt_menu, &agent_name,
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
                                            if !start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt(trigger)).await
                                            {
                                                worker_dead(&mut chat);
                                                break;
                                            }
                                            running = true;
                                            follow = true;
                                            if chat.agent == "plan" {
                                                chat.plan_submitted = true;
                                            }
                                            chat.begin_turn();
                                        }
                                    }
                                } else if running {
                                    if let Ok(seq) = store
                                        .admit_input(&mk_input(&session_id, Delivery::Queue, &clean))
                                        .await
                                    {
                                        queue_items.push((seq, clean.clone()));
                                    }
                                } else {
                                    push_user(&mut chat, &mut history, &mut hist_idx, &text);
                                    chat.context_used += estimate(&clean) as u64;
                                    if !start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt(clean)).await
                                    {
                                        worker_dead(&mut chat);
                                        break;
                                    }
                                    running = true;
                                    follow = true;
                                    if chat.agent == "plan" {
                                        chat.plan_submitted = true;
                                    }
                                    chat.begin_turn();
                                }
                            }
                            KeyAction::Steer(text) => {
                                let (clean, _unresolved) = resolve_and_warn(
                                    &text, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir, &skill_handle, &mut chat,
                                );
                                let clean = clean.trim();
                                if !clean.is_empty() {
                                    if let Ok(seq) = store.admit_input(&mk_input(&session_id, Delivery::Steer, clean)).await {
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
                                    if let Ok(seq) = store.admit_input(&mk_input(&session_id, Delivery::Steer, &trigger)).await {
                                        chat.steer_items.push((seq, trigger));
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
                                    if let Ok(seq) = store.admit_input(&mk_input(&session_id, Delivery::Queue, clean)).await {
                                        queue_items.push((seq, clean.to_string()));
                                    }
                                } else if let Some(skill_name) = active_skill.as_deref() {
                                    // Pure-skill submit (only a `{$name}` token,
                                    // no text): admit the skill trigger to the
                                    // queue so the active skill is acted on
                                    // instead of being silently dropped.
                                    let trigger = skill_trigger(skill_name);
                                    if let Ok(seq) = store.admit_input(&mk_input(&session_id, Delivery::Queue, &trigger)).await {
                                        queue_items.push((seq, trigger));
                                    }
                                }
                                follow = true;
                            }
                            KeyAction::SwitchAgent(name) => {
                                if matches!(
                                    app_loop::handle_switch_agent(
                                        name, &mut chat, &mut running, &mut follow, &mut input,
                                        &mut cursor_idx, &mut pending_handoff, &mut mode_flash,
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
                                pending_handoff = None;
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
                                        *skill_handle.lock().unwrap() = Some(body);
                                    }
                                    None => {
                                        active_skill = None;
                                        active_skill_body = None;
                                        sys_tokens = sys_tokens_for(&agent_name, &workdir, None);
                                        *skill_handle.lock().unwrap() = None;
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
                                pending_handoff = None; // cancel = explicit interrupt, no auto-handoff
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
                            if running {
                                cancel.cancel();
                                cancelled = true;
                                drain_pending = true;
                            } else {
                                start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt(String::new()))
                                    .await;
                                running = true;
                                chat.begin_turn();
                            }
                            follow = true;
                        }
                    }
                    Event::Resize(_, _) => {
                        // The input arm above already set `dirty=true`, so the
                        // next frame re-renders and refreshes the persisted
                        // `hits`. Also tell ratatui the size changed so its diff
                        // buffer matches the new layout (prevents glitches and
                        // keeps the persisted hit-rects valid after resize).
                        let _ = terminal.autoresize();
                    }
                    Event::Paste(pasted) => {
                        // Modal-priority paste routing (mirrors Event::Key).
                        if let app_loop::LoopFlow::Redraw = app_loop::route_paste(
                            &pasted, task_picker.is_some(), cache_salt_menu.is_some(),
                            &mut model_menu, &mut command_menu, &mut input,
                            &mut cursor_idx, &workdir,
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
                    &cmd_tx, &mut cancel, &mut pending_handoff, &mut evt_rx,
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
            }
            _ = body_ticker.tick() => {
                body_refresh_pending = true;
            }
        }
    }

    drop(cmd_tx);
    // The cancel issued on Quit should make the worker finish promptly. As a
    // last-resort guard against a tool/subagent that ignores cancellation,
    // bound the wait so the terminal is restored (TerminalGuard::drop leaves
    // the alt-screen) instead of freezing indefinitely on a blocked worker.
    let _ = tokio::time::timeout(Duration::from_secs(5), worker).await;
    Ok(session_id)
}

pub(crate) use crate::app_helpers::{
    clear_pending_inputs, data_dir_for, handle_mouse, mk_input, pre_key_intercept,
    push_user, resolve_and_warn, skill_trigger, start_turn, sys_tokens_for, worker_dead,
    MouseOutcome,
};

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
