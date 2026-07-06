use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent, KeyModifiers,
    MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use opencode_core::{discover_skills, resolve_agent, Config};
use opencode_llm::{estimate, ChatClient, ChatStream};
use opencode_session::{SessionEvent, SessionState};
use opencode_store::{Delivery, LibsqlStore, SessionInput, Store};
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::chat::ChatView;
use crate::command::{handle_command_key, CommandMenu, CommandOutcome, SlashAction};
use crate::worker::{process_cmd, rebind_session, gate_compact, CompactGate, UiCmd, UiEvent};
use crate::composer;
use crate::menu::SkillMenu;
use crate::model_menu::{handle_model_key, ModelMenu, ModelOutcome};
use crate::render::{in_rect, render, MouseHits, Term};
use crate::task::{handle_task_key, TaskOutcome, TaskPicker};
use crate::TuiOpts;

/// Double-Esc window: two Esc presses within this interval cancel the run.
const ESC_CANCEL_WINDOW_MS: u64 = 350;

/// Animation tick rate for the running spinner.
const ANIM_TICK_MS: u64 = 300;

pub async fn run(opts: &TuiOpts) -> Result<()> {
    let workdir = opts.workdir.clone().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let mut config = Config::load(&workdir)?;
    if let Some(m) = &opts.model { config.model = m.clone(); }
    let client: Arc<dyn ChatStream> = Arc::new(ChatClient::new(&config.provider.base_url, &config.api_key()?)?);
    let agent_name = opts.agent.clone().unwrap_or_else(|| config.agent.default.clone());
    let agent = resolve_agent(&agent_name).or_else(|| resolve_agent("act")).context("agent")?;

    let session_id = opencode_session::runner::new_id();
    let context_limit = config.context_limit();
    let model_label = config.model_id().to_string();

    let store: Arc<dyn Store> = {
        let data_dir = data_dir_for(&workdir);
        tokio::fs::create_dir_all(&data_dir).await.ok();
        Arc::new(LibsqlStore::open(data_dir.join("opencode.db")).await?)
    };

    let session = SessionState::new(
        session_id.clone(),
        agent,
        config.clone(),
        client.clone(),
        workdir.clone(),
    )
    .with_store(store.clone());

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    // Best-effort Kitty keyboard enhancement so Shift+Enter can be
    // distinguished from Enter. Silently ignored on terminals that don't
    // support it.
    {
        use crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
        let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
        let _ = execute!(stdout, PushKeyboardEnhancementFlags(flags));
    }
    execute!(stdout, EnterAlternateScreen, SetCursorStyle::SteadyBar, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Term::new(backend)?;

    let result = run_app(&mut terminal, session, store, session_id, context_limit, model_label, workdir, config, client).await;

    disable_raw_mode()?;
    {
        use crossterm::event::PopKeyboardEnhancementFlags;
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
    execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
    result
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

    let mut chat = crate::chat::ChatView { agent: session.agent.name.clone(), ..Default::default() };
    let mut input = String::new();
    let mut cursor_idx: usize = 0;
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut local_queue: VecDeque<String> = VecDeque::new();
    let mut running = false;
    let mut show_help = false;
    let mut scroll: u16 = 0;
    let mut follow = true;
    let mut context_used: u64 = 0;
    let mut sys_tokens: u64 = sys_tokens_for(session.agent.name.as_str(), &workdir, None);
    let mut steer_items: Vec<String> = Vec::new();
    let mut queue_items: Vec<String> = Vec::new();
    let mut skill_menu: Option<SkillMenu> = None;
    let mut task_picker: Option<TaskPicker> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut model_menu: Option<ModelMenu> = None;
    let mut active_skill: Option<String> = None;
    let mut anim_tick: u32 = 0;
    let mut last_esc: Option<Instant> = None;
    // Per-session UI state snapshots — saved on `/task` switch, restored on return.
    let mut session_states: std::collections::HashMap<String, crate::session_ui::SessionUiState> =
        std::collections::HashMap::new();

    let (mut cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(512);

    let worker = tokio::spawn(async move {
        let mut sess = session;
        while let Some(cmd) = cmd_rx.recv().await {
            if process_cmd(cmd, &mut sess, &evt_tx).await { break; }
        }
    });

    let mut events = EventStream::new();
    let mut anim_ticker = tokio::time::interval(Duration::from_millis(ANIM_TICK_MS));
    loop {
        let agent_name = chat.agent.clone();
        let status = chat.status.clone();
        // Compose status-bar model label with reasoning-effort badge (e.g.
        // "glm-5.2 \u{00b7}high") so the active thinking depth is visible.
        let status_model = match &config.reasoning_effort {
            Some(e) if !e.trim().is_empty() => format!("{model_label} \u{00b7}{e}"),
            _ => model_label.clone(),
        };
        let mut hits = MouseHits { jump_btn: None, body: None };
        render(
            terminal,
            &chat,
            &input,
            cursor_idx,
            &agent_name,
            running,
            show_help,
            context_used,
            sys_tokens,
            context_limit,
            &status_model,
            &workdir,
            &status,
            &steer_items,
            &queue_items,
            &mut scroll,
            follow,
            anim_tick,
            active_skill.as_deref(),
            skill_menu.as_ref(),
            task_picker.as_ref(),
            command_menu.as_ref(),
            model_menu.as_ref(),
            &mut hits,
        )?;

        tokio::select! {
            maybe_evt = events.next() => {
                if let Some(Ok(ev)) = maybe_evt {
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
                                                opencode_session::runner::new_id()
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
                                            running, &chat, &history, scroll, follow, context_used, sys_tokens, &steer_items, &queue_items, &active_skill,
                                        ));
                                        // Restore or create the target session's UI state.
                                        let restored = session_states.remove(&new_session_id);
                                        if let Some(st) = restored {
                                            chat = st.chat;
                                            history = st.history;
                                            scroll = st.scroll;
                                            follow = st.follow;
                                            context_used = st.context_used;
                                            sys_tokens = st.sys_tokens;
                                            steer_items = st.steer_items;
                                            queue_items = st.queue_items;
                                            active_skill = st.active_skill;
                                            running = false; // worker was killed on switch
                                        } else {
                                            // Fresh state for a new or resumed session.
                                            if let crate::task::TaskPick::Resume(_) = pick {
                                                chat = crate::session_ui::replay_into_chat(&agent_name_for_tokens, &resumed_messages);
                                            } else {
                                                chat = ChatView { agent: agent_name_for_tokens.clone(), ..Default::default() };
                                            }
                                            scroll = 0; follow = true;
                                            context_used = 0;
                                            sys_tokens = sys_tokens_for(&agent_name_for_tokens, &workdir_for_tokens, None);
                                            steer_items.clear(); queue_items.clear();
                                            active_skill = None; running = false;
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
                                    }
                                    TaskOutcome::Quit => { let _ = cmd_tx.send(UiCmd::Quit).await; break; }
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
                                                        if let Ok(new_client) = opencode_llm::ChatClient::new(&reloaded.provider.base_url, &api_key) {
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
                                        let sessions = store.list_sessions(&opencode_store::SessionFilter::default())
                                            .await.unwrap_or_default();
                                        task_picker = Some(TaskPicker::new(sessions));
                                    }
                                    CommandOutcome::Dispatch(SlashAction::Model) => {
                                        model_menu = Some(ModelMenu::new(&config));
                                    }
                                    CommandOutcome::Dispatch(SlashAction::Compact) => {
                                        match gate_compact(running) {
                                            CompactGate::Run => {
                                                let _ = cmd_tx.send(UiCmd::Compact).await;
                                                running = true;
                                                follow = true;
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
                                    if running {
                                        let _ = store.admit_input(&mk_input(&session_id, Delivery::Queue, &text)).await;
                                        queue_items.push(text.clone());
                                        chat.push_marker(Line::from(Span::styled(
                                            format!("[queued] {text}"), Style::default().fg(Color::Yellow))));
                                    } else {
                                        push_user(&mut chat, &mut history, &mut hist_idx, &text);
                                        context_used += estimate(&text) as u64;
                                        let _ = cmd_tx.send(UiCmd::Prompt(text)).await;
                                        running = true;
                                        follow = true;
                                    }
                                }
                                KeyAction::Steer(text) => {
                                    let _ = store.admit_input(&mk_input(&session_id, Delivery::Steer, &text)).await;
                                    steer_items.push(text.clone());
                                    chat.push_marker(Line::from(Span::styled(
                                        format!("\u{21b3} steer: {text}"), Style::default().fg(Color::Blue))));
                                    follow = true;
                                }
                                KeyAction::Queue(text) => {
                                    let _ = store.admit_input(&mk_input(&session_id, Delivery::Queue, &text)).await;
                                    queue_items.push(text.clone());
                                    chat.push_marker(Line::from(Span::styled(
                                        format!("[queued] {text}"), Style::default().fg(Color::Yellow))));
                                    follow = true;
                                }
                                KeyAction::SwitchAgent(name) => {
                                    let plan_to_act = chat.agent == "plan" && name == "act" && !running;
                                    sys_tokens = sys_tokens_for(&name, &workdir, active_skill.as_deref());
                                    if plan_to_act && !chat.blocks.is_empty() {
                                        let _ = cmd_tx.send(UiCmd::SwitchAndStart(name)).await;
                                        running = true;
                                        follow = true;
                                    } else {
                                        let _ = cmd_tx.send(UiCmd::SwitchAgent(name)).await;
                                    }
                                }
                                KeyAction::SetSkill(opt) => {
                                    match opt {
                                        Some((name, body)) => {
                                            active_skill = Some(name.clone());
                                            sys_tokens = sys_tokens_for(&agent_name, &workdir, Some(&body));
                                            let _ = cmd_tx.send(UiCmd::SetSkill(Some(body))).await;
                                        }
                                        None => {
                                            active_skill = None;
                                            sys_tokens = sys_tokens_for(&agent_name, &workdir, None);
                                            let _ = cmd_tx.send(UiCmd::SetSkill(None)).await;
                                        }
                                    }
                                }
                                KeyAction::Cancel => {
                                    cancel.cancel();
                                    chat.push_marker(Line::from(Span::styled(
                                        "[interrupted] stopping…", Style::default().fg(Color::Yellow))));
                                    running = false;
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
                            match m.kind {
                                MouseEventKind::Down(MouseButton::Left) => {
                                    if let Some(r) = hits.jump_btn {
                                        if in_rect(r, m.column, m.row) {
                                            follow = true;
                                        }
                                    }
                                }
                                MouseEventKind::ScrollUp => {
                                    if let Some(r) = hits.body {
                                        if in_rect(r, m.column, m.row) {
                                            scroll = scroll.saturating_sub(3);
                                            follow = false;
                                        }
                                    }
                                }
                                MouseEventKind::ScrollDown => {
                                    if let Some(r) = hits.body {
                                        if in_rect(r, m.column, m.row) {
                                            let visible_h = r.height.saturating_sub(2) as usize;
                                            let inner_w = r.width.saturating_sub(2);
                                            let total_rows = Paragraph::new(chat.flatten())
                                                .wrap(Wrap { trim: false })
                                                .line_count(inner_w);
                                            let max_rows = total_rows.saturating_sub(visible_h);
                                            scroll = scroll.saturating_add(3);
                                            if (scroll as usize) >= max_rows {
                                                follow = true;
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some(ev) = evt_rx.recv() => {
                match ev {
                    UiEvent::Session(sev) => {
                        if let SessionEvent::TranscriptReset(msgs) = &sev {
                            let agent = chat.agent.clone();
                            chat = crate::session_ui::replay_into_chat(&agent, msgs);
                        } else {
                            track_context(&sev, &mut context_used);
                            chat.apply(&sev);
                        }
                        if matches!(sev, SessionEvent::Done | SessionEvent::Error(_)) {
                            running = false;
                            steer_items.clear();
                            queue_items.clear();
                        }
                    }
                    UiEvent::TurnDone => {
                        running = false;
                        if let Some(next) = local_queue.pop_front() {
                            push_user(&mut chat, &mut history, &mut hist_idx, &next);
                            let _ = cmd_tx.send(UiCmd::Prompt(next)).await;
                            running = true;
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

fn mk_input(session_id: &str, delivery: Delivery, prompt: &str) -> SessionInput {
    SessionInput {
        id: opencode_session::runner::new_id(),
        session_id: session_id.to_string(),
        delivery,
        prompt: prompt.to_string(),
        admitted_seq: 0,
        promoted_seq: None,
    }
}

fn track_context(ev: &SessionEvent, used: &mut u64) {
    match ev {
        SessionEvent::TextDelta(t) | SessionEvent::ReasoningDelta(t) => *used += estimate(t) as u64,
        SessionEvent::ToolEnd { output, .. } => *used += estimate(output) as u64,
        SessionEvent::SubagentEnd { summary, .. } => *used += estimate(summary) as u64,
        SessionEvent::Compaction(c) => *used = estimate(c) as u64,
        _ => {}
    }
}

/// Estimated tokens of the system prompt that will accompany every request:
/// `agent.prompt + environment block + active skill`. Tracked separately from
/// `context_used` (which only sums the streamed transcript and resets on
/// compaction) so the context meter reflects the real request size.
pub(crate) fn sys_tokens_for(agent_name: &str, workdir: &Path, skill: Option<&str>) -> u64 {
    let agent = match resolve_agent(agent_name) {
        Some(a) => a,
        None => return 0,
    };
    let text = opencode_session::prompt::build_system(&agent, workdir, skill).text();
    estimate(&text) as u64
}

fn push_user(chat: &mut ChatView, history: &mut Vec<String>, hist_idx: &mut Option<usize>, text: &str) {
    history.push(text.to_string());
    *hist_idx = None;
    chat.push_marker(Line::from(Span::styled(
        format!("user: {text}"), Style::default().add_modifier(Modifier::BOLD))));
    chat.push_marker(Line::from(""));
}

#[derive(Debug, PartialEq)]
pub(crate) enum KeyAction {
    None,
    Submit(String),
    Steer(String),
    Queue(String),
    SwitchAgent(String),
    Cancel,
    SetSkill(Option<(String, String)>),
    OpenCommand,
    Quit,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_key(
    k: KeyEvent,
    input: &mut String,
    cursor_idx: &mut usize,
    history: &[String],
    hist_idx: &mut Option<usize>,
    running: bool,
    agent: &str,
    show_help: &mut bool,
    scroll: &mut u16,
    follow: &mut bool,
    last_esc: &mut Option<Instant>,
    skill_menu: &mut Option<SkillMenu>,
    active_skill: Option<&str>,
) -> KeyAction {
    // Modal skill picker: intercept all keys while open.
    if skill_menu.is_some() {
        return match crate::menu::handle_menu_key(skill_menu, k) {
            crate::menu::MenuOutcome::Quit => KeyAction::Quit,
            crate::menu::MenuOutcome::Pick(opt) => KeyAction::SetSkill(opt),
            crate::menu::MenuOutcome::Idle => KeyAction::None,
        };
    }
    // Alt+Tab (and Ctrl+T fallback) switches act <-> plan mode.
    if k.modifiers.contains(KeyModifiers::ALT) && matches!(k.code, KeyCode::Tab | KeyCode::BackTab) {
        let next = if agent == "plan" { "act" } else { "plan" };
        return KeyAction::SwitchAgent(next.into());
    }

    if k.modifiers.contains(KeyModifiers::CONTROL) {
        match k.code {
            KeyCode::Char('c') | KeyCode::Char('d') => return KeyAction::Quit,
            // Fallback mode switch for terminals that swallow Alt+Tab.
            KeyCode::Char('t') => {
                let next = if agent == "plan" { "act" } else { "plan" };
                return KeyAction::SwitchAgent(next.into());
            }
            KeyCode::Char('h') => { *show_help = !*show_help; return KeyAction::None; }
            KeyCode::Char('n') => { move_hist(history, hist_idx, input, cursor_idx, 1); return KeyAction::None; }
            KeyCode::Char('p') => { move_hist(history, hist_idx, input, cursor_idx, -1); return KeyAction::None; }
            KeyCode::Char('u') => { *scroll = scroll.saturating_sub(10); *follow = false; return KeyAction::None; }
            _ => return KeyAction::None,
        }
    }
    match k.code {
        KeyCode::BackTab => {
            // Shift+Tab = primary mode switch (codex-cli style).
            let next = if agent == "plan" { "act" } else { "plan" };
            KeyAction::SwitchAgent(next.into())
        }
        KeyCode::Enter => {
            // Shift+Enter or Alt+Enter inserts a newline (multi-line input).
            if k.modifiers.contains(KeyModifiers::SHIFT) {
                let (s, i) = composer::insert_newline(input, *cursor_idx);
                *input = s; *cursor_idx = i;
                return KeyAction::None;
            }
            if input.trim().is_empty() { return KeyAction::None; }
            let text = input.trim().to_string();
            input.clear(); *cursor_idx = 0; *hist_idx = None;
            // Enter = Steer when running (strong intervention, promoted at
            // turn boundary); normal submit when idle.
            if running { KeyAction::Steer(text) } else { KeyAction::Submit(text) }
        }
        KeyCode::Tab => {
            // Tab = follow-up (queue) when running; normal submit when idle.
            if input.trim().is_empty() { return KeyAction::None; }
            let text = input.trim().to_string();
            input.clear(); *cursor_idx = 0; *hist_idx = None;
            if running { KeyAction::Queue(text) } else { KeyAction::Submit(text) }
        }
        KeyCode::Esc => {
            // 1) If help is open, Esc just closes it.
            if *show_help {
                *show_help = false;
                return KeyAction::None;
            }
            // 2) Double-Esc within the window while running => hard-abort.
            let now = Instant::now();
            let is_double = running
                && last_esc
                    .map(|t| now.duration_since(t) < Duration::from_millis(ESC_CANCEL_WINDOW_MS))
                    .unwrap_or(false);
            if is_double {
                *last_esc = None;
                KeyAction::Cancel
            } else {
                *last_esc = Some(now);
                input.clear(); *cursor_idx = 0; *hist_idx = None;
                KeyAction::None
            }
        }
        KeyCode::Up => {
            if input.contains('\n') {
                *cursor_idx = composer::move_cursor_vertical(input, *cursor_idx, -1);
            } else {
                move_hist(history, hist_idx, input, cursor_idx, -1);
            }
            KeyAction::None
        }
        KeyCode::Down => {
            if input.contains('\n') {
                *cursor_idx = composer::move_cursor_vertical(input, *cursor_idx, 1);
            } else {
                move_hist(history, hist_idx, input, cursor_idx, 1);
            }
            KeyAction::None
        }
        KeyCode::Left => { *cursor_idx = cursor_idx.saturating_sub(1); KeyAction::None }
        KeyCode::Right => { *cursor_idx = (*cursor_idx + 1).min(input.chars().count()); KeyAction::None }
        KeyCode::Home => { *cursor_idx = 0; KeyAction::None }
        KeyCode::End => { *cursor_idx = input.chars().count(); KeyAction::None }
        KeyCode::PageUp => { *scroll = scroll.saturating_sub(20); *follow = false; KeyAction::None }
        KeyCode::PageDown => { *follow = true; KeyAction::None }
        KeyCode::Backspace => {
            if let Some((s, i)) = composer::backspace(input, *cursor_idx) {
                *input = s; *cursor_idx = i;
            }
            KeyAction::None
        }
        KeyCode::Char(c) => {
            // Fallback quit for terminals/crossterm configs that deliver Ctrl+C
            // (ETX, 0x03) and Ctrl+D (EOT, 0x04) as raw control chars without the
            // CONTROL modifier flag (the Ctrl-block match above would miss them).
            if c == '\u{3}' || c == '\u{4}' {
                return KeyAction::Quit;
            }
            if c == '$' && input.is_empty() && *cursor_idx == 0 {
                *skill_menu = Some(SkillMenu::new(discover_skills(), active_skill.is_some()));
                return KeyAction::None;
            }
            // `/` on empty input opens the slash-command picker. Bare `/` +
            // Enter defaults to /task (first row) for muscle memory.
            if c == '/' && input.is_empty() && *cursor_idx == 0 {
                return KeyAction::OpenCommand;
            }
            let (s, i) = composer::insert_char(input, *cursor_idx, c);
            *input = s; *cursor_idx = i;
            KeyAction::None
        }
        _ => KeyAction::None,
    }
}

fn move_hist(history: &[String], hist_idx: &mut Option<usize>, input: &mut String, cursor_idx: &mut usize, delta: i32) {
    if history.is_empty() { return; }
    let cur = hist_idx.unwrap_or(history.len());
    let next = (cur as i32 + delta).clamp(0, history.len() as i32) as usize;
    if next < history.len() {
        *hist_idx = Some(next);
        *input = history[next].clone();
    } else {
        *hist_idx = None;
        input.clear();
    }
    *cursor_idx = input.chars().count();
}

/// Mouse hit-targets exported by `render` for the event loop to test clicks
/// and wheel scrolls against. Recomputed every frame.
fn data_dir_for(workdir: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    workdir.hash(&mut h);
    let digest = h.finish();
    let mut base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    base.push("opencode");
    base.push(format!("{digest:x}"));
    base
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
