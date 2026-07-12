use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::Event;
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{estimate, ChatClient, ChatStream};
use opencoder_session::{SessionEvent, SessionState};
use opencoder_store::{Delivery, LibsqlStore, Store};
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::chat::ChatView;
use crate::command::{handle_command_key, CommandMenu, CommandOutcome, SlashAction};
use crate::input::spawn_input_pump;
use crate::key_handler::{handle_key, KeyAction};
use crate::menu::SkillMenu;
use crate::model_menu::{handle_model_key, ModelMenu, ModelOutcome};
use crate::render::{render, MouseHits, Term};
use crate::task::{handle_task_key, TaskOutcome, TaskPicker};
use crate::terminal::TerminalGuard;
use crate::worker::{
    gate_clear_all, gate_compact, process_cmd, rebind_session, ClearAllGate, CompactGate, UiCmd,
    UiEvent,
};
use crate::TuiOpts;

/// Animation tick rate for the running spinner.
const ANIM_TICK_MS: u64 = 300;
/// How long the plan/act switch flash stays visible, in anim ticks (~300ms each).
const MODE_FLASH_TICKS: u32 = 5;

/// Whether a transient flash started at `start` is still visible at `now`,
/// given a lifetime of `ticks` anim ticks. Uses wrapping subtraction so it
/// stays correct across the u32 wraparound of `anim_tick`.
pub(crate) fn flash_visible(start: u32, now: u32, ticks: u32) -> bool {
    now.wrapping_sub(start) < ticks
}

pub async fn run(opts: &TuiOpts) -> Result<()> {
    let workdir = opts
        .workdir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let mut config = Config::load(&workdir)?;
    if let Some(m) = &opts.model {
        config.model = m.clone();
    }
    let client: Arc<dyn ChatStream> = Arc::new(ChatClient::new(
        &config.provider.base_url,
        &config.api_key()?,
    )?);
    let agent_name = opts
        .agent
        .clone()
        .unwrap_or_else(|| config.agent.default.clone());
    let agent = resolve_agent(&agent_name)
        .or_else(|| resolve_agent("act"))
        .context("agent")?;

    let session_id = opencoder_session::runner::new_id();
    let context_limit = config.context_limit();
    let model_label = config.model_id().to_string();

    let store: Arc<dyn Store> = {
        let data_dir = data_dir_for(&workdir);
        tokio::fs::create_dir_all(&data_dir).await.ok();
        Arc::new(LibsqlStore::open(data_dir.join("opencoder.db")).await?)
    };

    let session = SessionState::new(
        session_id.clone(),
        agent,
        config.clone(),
        client.clone(),
        workdir.clone(),
    )
    .with_store(store.clone());

    // Terminal enter/restore is RAII: `TerminalGuard`'s Drop — and the panic
    // hook it installs — restore raw/alt-screen/mouse/kitty state on ANY exit
    // path (normal return, `?` error, or a panic that unwinds). This removes
    // the old "cleanup only ran on the happy path" trap that bricked the
    // terminal on any panic, leaving the user with a frozen last frame, no
    // echo, and ineffective Ctrl+C/D.
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Term::new(backend)?;

    run_app(
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
    .await
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
) -> Result<()> {
    // Wire a cancellation token into the session so double-Esc can hard-abort
    // the running turn (mid-stream / mid-tool). The UI keeps a clone to signal.
    // `mut`: reassigned by `rebind_session` on every `/task` session switch.
    let mut cancel = CancellationToken::new();
    let session = session.with_cancel(cancel.clone());
    let mut skill_handle = session.skill_prompt.clone();

    let mut chat = crate::chat::ChatView {
        agent: session.agent.name.clone(),
        ..Default::default()
    };
    let mut input = String::new();
    let mut cursor_idx: usize = 0;
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut local_queue: VecDeque<String> = VecDeque::new();
    let mut running = false;
    let mut cancelled = false;
    let mut show_help = false;
    let mut scroll: u16 = 0;
    let mut follow = true;
    let mut sys_tokens: u64 = sys_tokens_for(session.agent.name.as_str(), &workdir, None);
    // Cached system-prompt tokens for the subagent currently being viewed.
    // Computed once on entry (ctx-switch click) to avoid per-frame rebuild.
    let mut subagent_sys: u64 = 0;
    let mut steer_items: Vec<String> = Vec::new();
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut skill_menu: Option<SkillMenu> = None;
    let mut task_picker: Option<TaskPicker> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut model_menu: Option<ModelMenu> = None;
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
    let mut skip_next_render = false;
    loop {
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
                        &chat,
                        agent_name.clone(),
                        agent_name.clone(),
                        chat.context_used,
                        sys_tokens,
                    ),
                }
            } else {
                (
                    &chat,
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
            _ => model_label.clone(),
        };
        let mut hits = MouseHits::default();
        if !skip_next_render {
            render(
                terminal,
                display_chat,
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
                &steer_items,
                &queue_items,
                &mut scroll,
                follow,
                anim_tick,
                mode_flash.as_ref().and_then(|(t, s)| {
                    if flash_visible(*s, anim_tick, MODE_FLASH_TICKS) {
                        Some(t.as_str())
                    } else {
                        None
                    }
                }),
                active_skill.as_deref(),
                skill_menu.as_ref(),
                task_picker.as_ref(),
                command_menu.as_ref(),
                model_menu.as_ref(),
                &mut hits,
                selection,
            )?;
        }
        skip_next_render = false;

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
                match ev {
                    Event::Key(k) => {
                        // Task picker modal: intercept all keys while open.
                        if task_picker.is_some() {
                            match handle_task_key(&mut task_picker, k) {
                                TaskOutcome::Pick(pick) => {
                                    // Perform session switch.
                                    let _ = cmd_tx.send(UiCmd::Quit).await;
                                    let new_session_id = match &pick {
                                        crate::task::TaskPick::New => {
                                            opencoder_session::runner::new_id()
                                        }
                                        crate::task::TaskPick::Resume(id) => id.clone(),
                                    };
                                    let new_agent = resolve_agent("act").context("agent")?;
                                    let new_config = Config::load(&workdir).unwrap_or_else(|_| config.clone());                                        let mut new_session = SessionState::new(
                                        new_session_id.clone(),
                                        new_agent,
                                        new_config,
                                        client.clone(),
                                        workdir.clone(),
                                    ).with_store(store.clone());
                                    new_session.model = model_label.clone();
                                    if let crate::task::TaskPick::Resume(ref id) = pick {
                                        if let Ok(msgs) = store.load_messages(id).await {
                                            new_session.messages = msgs;
                                        }
                                    }
                                    let new_cancel = CancellationToken::new();
                                    let new_session = new_session.with_cancel(new_cancel.clone());
                                    let new_skill_handle = new_session.skill_prompt.clone();
                                    let resumed_messages = if let crate::task::TaskPick::Resume(_) = &pick {
                                        new_session.messages.clone()
                                    } else {
                                        Vec::new()
                                    };
                                    let (ntx, nrx) = mpsc::channel::<UiEvent>(512);
                                    let (n_cmd_tx, mut n_cmd_rx) = mpsc::channel::<UiCmd>(64);
                                    let session_for_worker = new_session;
                                    let agent_name_for_tokens = session_for_worker.agent.name.clone();
                                    let workdir_for_tokens = session_for_worker.working_dir.clone();
                                    tokio::spawn(async move {
                                        let mut sess = session_for_worker;
                                        while let Some(cmd) = n_cmd_rx.recv().await {
                                            if process_cmd(cmd, &mut sess, &ntx).await { break; }
                                        }
                                    });
                                    // Save current session's UI state before switching.
                                    session_states.insert(session_id.clone(), crate::session_ui::SessionUiState::snapshot(
                                        running, &chat, &history, scroll, follow, sys_tokens, &steer_items, &queue_items, &active_skill, &active_skill_body,
                                    ));
                                    // Restore or create the target session's UI state.
                                    let restored = session_states.remove(&new_session_id);
                                    if let Some(st) = restored {
                                        chat = st.chat;
                                        history = st.history;
                                        scroll = st.scroll;
                                        follow = st.follow;
                                        sys_tokens = st.sys_tokens;
                                        steer_items = st.steer_items;
                                        queue_items = st.queue_items;
                                        active_skill = st.active_skill;
                                        active_skill_body = st.active_skill_body;
                                        running = false; // worker was killed on switch
                                    } else {
                                        // Fresh state for a new or resumed session.
                                        if let crate::task::TaskPick::Resume(_) = pick {
                                            chat = crate::session_ui::replay_into_chat(&agent_name_for_tokens, &resumed_messages, &store, &new_session_id).await;
                                        } else {
                                            chat = ChatView { agent: agent_name_for_tokens.clone(), ..Default::default() };
                                        }
                                        scroll = 0; follow = true;
                                        sys_tokens = sys_tokens_for(&agent_name_for_tokens, &workdir_for_tokens, None);
                                        steer_items.clear(); queue_items.clear();
                                        active_skill = None; active_skill_body = None; running = false;
                                    }
                                    input.clear(); cursor_idx = 0; hist_idx = None;
                                    rebind_session(
                                        &mut cmd_tx,
                                        &mut evt_rx,
                                        &mut session_id,
                                        &mut cancel,
                                        n_cmd_tx,
                                        nrx,
                                        new_session_id,
                                        new_cancel,
                                    );
                                    // The freshly-spawned worker starts with no
                                    // skill prompt; re-sync the sticky skill so a
                                    // resumed session's active skill actually
                                    // applies to its turns.
                                    skill_handle = new_skill_handle;
                                    if let Some(body) = &active_skill_body {
                                        *skill_handle.lock().unwrap() = Some(body.clone());
                                    }
                                }
                                TaskOutcome::Quit => { let _ = cmd_tx.send(UiCmd::Quit).await; break; }
                                TaskOutcome::ClearAll { keep_session_id } => {
                                    // Refuse while a turn / subagent is in flight: a running
                                    // subagent's child session is still being written to, and
                                    // clearing would FK-violate its next append. Retry at idle.
                                    match gate_clear_all(running) {
                                        ClearAllGate::SkipRunning => {
                                            if let Some(p) = task_picker.as_mut() {
                                                p.reset_confirmation();
                                            }
                                            chat.push_marker(Line::from(Span::styled(
                                                "[task] clear busy \u{2014} retry when idle (subagents still running)",
                                                Style::default().fg(Color::Yellow),
                                            )));
                                        }
                                        ClearAllGate::Run => {
                                            let before = task_picker.as_ref().map(|p| p.deletable_count()).unwrap_or(0);
                                            match store.clear_other_sessions(&keep_session_id).await {
                                                Ok(n) => {
                                                    let sessions = store.list_sessions(&opencoder_store::SessionFilter::default())
                                                        .await.unwrap_or_default();
                                                    if let Some(p) = task_picker.as_mut() {
                                                        p.reset_sessions(sessions);
                                                    }
                                                    chat.push_marker(Line::from(Span::styled(
                                                        format!("[/task] cleared {n} of {before} task(s)"),
                                                        Style::default().fg(Color::Green),
                                                    )));
                                                }
                                                Err(e) => {
                                                    if let Some(p) = task_picker.as_mut() {
                                                        p.reset_confirmation();
                                                    }
                                                    chat.push_marker(Line::from(Span::styled(
                                                        format!("[/task] clear failed: {e:#}"),
                                                        Style::default().fg(Color::Red),
                                                    )));
                                                }
                                            }
                                        }
                                    }
                                }
                                TaskOutcome::Idle => {}
                            }
                            continue;
                        }
                        // `/model` modal: intercept all keys while open.
                        if model_menu.is_some() {
                            match handle_model_key(&mut model_menu, k) {
                                ModelOutcome::Save(patch) => {
                                    let json = patch.to_json();
                                    match Config::save(&workdir, &json) {
                                        Ok(path) => {
                                            match Config::load(&workdir) {
                                                Ok(reloaded) => {
                                                    model_label = reloaded.model_id().to_string();
                                                    context_limit = reloaded.context_limit();
                                                    // Rebuild the outer `client` too so subsequent
                                                    // `/task` new sessions pick up the new endpoint
                                                    // (the worker only swaps its own sess.client).
                                                    let api_key = reloaded.api_key().unwrap_or_default();
                                                    if let Ok(new_client) = opencoder_llm::ChatClient::new(&reloaded.provider.base_url, &api_key) {
                                                        client = Arc::new(new_client);
                                                    }
                                                    config = reloaded.clone();
                                                    let _ = cmd_tx.send(UiCmd::ReloadConfig(reloaded)).await;
                                                    chat.push_marker(Line::from(Span::styled(
                                                        format!("[/model] saved \u{2192} {}", path.display()),
                                                        Style::default().fg(Color::Green))));
                                                }
                                                Err(e) => {
                                                    chat.push_marker(Line::from(Span::styled(
                                                        format!("[/model] reload failed: {e:#}"),
                                                        Style::default().fg(Color::Red))));
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            chat.push_marker(Line::from(Span::styled(
                                                format!("[/model] save failed: {e:#}"),
                                                Style::default().fg(Color::Red))));
                                        }
                                    }
                                }
                                ModelOutcome::Cancel | ModelOutcome::Idle => {}
                                ModelOutcome::Quit => { let _ = cmd_tx.send(UiCmd::Quit).await; break; }
                            }
                            continue;
                        }
                        // `/` command picker: intercept all keys while open.
                        if command_menu.is_some() {
                            let (outcome, quit) = handle_command_key(&mut command_menu, k);
                            if quit { let _ = cmd_tx.send(UiCmd::Quit).await; break; }
                            match outcome {
                                CommandOutcome::Dispatch(SlashAction::Task) => {
                                    let sessions = store.list_sessions(&opencoder_store::SessionFilter::default())
                                        .await.unwrap_or_default();
                                    task_picker = Some(TaskPicker::new(sessions, session_id.clone()));
                                }
                                CommandOutcome::Dispatch(SlashAction::Model) => {
                                    model_menu = Some(ModelMenu::new(&config));
                                }
                                CommandOutcome::Dispatch(SlashAction::Compact) => {
                                    match gate_compact(running) {
                                        CompactGate::Run => {
                                            if !start_turn(&cmd_tx, &mut cancel, UiCmd::Compact)
                                                .await
                                            {
                                                worker_dead(&mut chat);
                                                break;
                                            }
                                            running = true;
                                            follow = true;
                                            chat.status.clear();
                                        }
                                        CompactGate::SkipRunning => {
                                            chat.push_marker(Line::from(Span::styled(
                                                "[compact] busy \u{2014} retry when idle",
                                                Style::default().fg(Color::Yellow),
                                            )));
                                        }
                                    }
                                }
                                CommandOutcome::Idle => {}
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
                            active_skill.as_deref(),
                        ) {
                            KeyAction::Submit(text) => {
                                let (clean, _unresolved) = resolve_and_warn(
                                    &text, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir, &skill_handle, &mut chat,
                                );
                                let clean = clean.trim().to_string();
                                if clean.is_empty() {
                                    if active_skill.is_some() {
                                        chat.push_marker(Line::from(Span::styled(
                                            format!("[skill: {}]", active_skill.as_deref().unwrap_or("")),
                                            Style::default().fg(Color::Yellow))));
                                        if !running {
                                            // Skill-only submit: start a drain-mode turn
                                            // (empty prompt) so the model reads the injected
                                            // skill body and acts on it immediately.
                                            if !start_turn(
                                                &cmd_tx,
                                                &mut cancel,
                                                UiCmd::Prompt(String::new()),
                                            )
                                            .await
                                            {
                                                worker_dead(&mut chat);
                                                break;
                                            }
                                            running = true;
                                            follow = true;
                                            chat.status.clear();
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
                                    push_user(&mut chat, &mut history, &mut hist_idx, &clean);
                                    chat.context_used += estimate(&clean) as u64;
                                    if !start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt(clean)).await
                                    {
                                        worker_dead(&mut chat);
                                        break;
                                    }
                                    running = true;
                                    follow = true;
                                    chat.status.clear();
                                }
                            }
                            KeyAction::Steer(text) => {
                                let (clean, _unresolved) = resolve_and_warn(
                                    &text, &mut active_skill, &mut active_skill_body,
                                    &mut sys_tokens, &agent_name, &workdir, &skill_handle, &mut chat,
                                );
                                let clean = clean.trim();
                                if !clean.is_empty() {
                                    let _ = store.admit_input(&mk_input(&session_id, Delivery::Steer, clean)).await;
                                    steer_items.push(clean.to_string());
                                    chat.push_marker(Line::from(Span::styled(
                                        format!("\u{21b3} steer: {clean}"), Style::default().fg(Color::Blue))));
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
                                }
                                follow = true;
                            }
                            KeyAction::SwitchAgent(name) => {
                                mode_flash = Some((format!("\u{2192} {name} mode"), anim_tick));
                                let plan_to_act = chat.agent == "plan" && name == "act" && !running;
                                sys_tokens = sys_tokens_for(&name, &workdir, active_skill_body.as_deref());
                                if plan_to_act && !chat.blocks.is_empty() {
                                    if !start_turn(&cmd_tx, &mut cancel, UiCmd::SwitchAndStart(name))
                                        .await
                                    {
                                        worker_dead(&mut chat);
                                        break;
                                    }
                                    running = true;
                                    follow = true;
                                    chat.status.clear();
                                } else {
                                    let _ = cmd_tx.send(UiCmd::SwitchAgent(name)).await;
                                }
                            }
                            KeyAction::SetSkill(opt) => {
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
                            }
                            KeyAction::Cancel => {
                                cancel.cancel();
                                chat.push_marker(Line::from(Span::styled(
                                    "[interrupted] stopping…", Style::default().fg(Color::Yellow))));
                                running = false;
                                cancelled = true;
                                follow = true;
                            }
                            KeyAction::OpenCommand => {
                                command_menu = Some(CommandMenu::new());
                            }
                            KeyAction::Quit => { let _ = cmd_tx.send(UiCmd::Quit).await; break; }
                            KeyAction::None => {}
                        }
                    }
                    Event::Mouse(m) => {
                        handle_mouse(
                            m,
                            &hits,
                            &mut scroll,
                            &mut follow,
                            &mut selection,
                            &mut chat,
                            &mut subagent_focus,
                            &mut parent_scroll,
                            &mut parent_follow,
                            &mut subagent_sys,
                            &workdir,
                            &mut queue_items,
                            &session_id,
                            store.as_ref(),
                        )
                        .await;
                    }
                    _ => {}
                }
            }
            maybe_ev = evt_rx.recv() => {
                let ev = match maybe_ev {
                    Some(ev) => ev,
                    None => {
                        worker_dead(&mut chat);
                        break;
                    }
                };
                match ev {
                    UiEvent::Session(sev) => {
                        if let SessionEvent::TranscriptReset(msgs) = &sev {
                            let agent = chat.agent.clone();
                            chat = crate::session_ui::replay_into_chat(&agent, msgs, &store, &session_id).await;
                        } else {
                            chat.apply(&sev);
                            if matches!(sev, SessionEvent::ReasoningDelta(_))
                                && chat.last_thinking_collapsed()
                            {
                                skip_next_render = true;
                            }
                        }
                        if let SessionEvent::QueueConsumed { seq } = &sev {
                            queue_items.retain(|(s, _)| s != seq);
                        }
                        if matches!(sev, SessionEvent::Done | SessionEvent::Error(_)) {
                            if cancelled {
                                // Stale event from a cancelled turn — consume without
                                // affecting running or clearing items belonging to a
                                // potentially-new turn.
                                cancelled = false;
                            } else {
                                running = false;
                                steer_items.clear();
                                queue_items.clear();
                            }
                        }
                    }
                    UiEvent::TurnDone => {
                        if cancelled {
                            cancelled = false;
                        } else {
                            running = false;
                        }
                        if let Some(next) = local_queue.pop_front() {
                            push_user(&mut chat, &mut history, &mut hist_idx, &next);
                            if !start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt(next)).await {
                                worker_dead(&mut chat);
                                break;
                            }
                            running = true;
                            chat.status.clear();
                        }
                    }
                }
            }
            _ = anim_ticker.tick() => {
                if running {
                    anim_tick = anim_tick.wrapping_add(1);
                }
            }
        }
    }

    drop(cmd_tx);
    let _ = worker.await;
    Ok(())
}

pub(crate) use crate::app_helpers::{
    data_dir_for, handle_mouse, mk_input, pre_key_intercept, push_user, resolve_and_warn,
    start_turn, sys_tokens_for, worker_dead,
};

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
