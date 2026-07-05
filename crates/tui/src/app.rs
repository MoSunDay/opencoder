use std::collections::VecDeque;
use std::io::Stdout;
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
use opencode_session::{run as run_session, SessionEvent, SessionState};
use opencode_store::{Delivery, LibsqlStore, SessionInput, Store};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::Frame;
use ratatui::Terminal;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::chat::ChatView;
use crate::composer;
use crate::fmt as fmtmod;
use crate::menu::{render_skill_popup, SkillMenu};
use crate::TuiOpts;

type Term = Terminal<CrosstermBackend<Stdout>>;

/// Context baseline subtracted from used/window so small sessions read ~0%.
const CONTEXT_BASELINE: u64 = 4_000;

/// Double-Esc window: two Esc presses within this interval cancel the run.
const ESC_CANCEL_WINDOW_MS: u64 = 350;

/// Animation tick rate for the running spinner.
const ANIM_TICK_MS: u64 = 300;

/// Braille spinner frames shown while a task is running.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

enum UiCmd {
    Prompt(String),
    SwitchAgent(String),
    SetSkill(Option<String>),
    Quit,
}

enum UiEvent {
    Session(SessionEvent),
    TurnDone,
}

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
        config,
        client,
        workdir.clone(),
    )
    .with_store(store.clone());

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, SetCursorStyle::SteadyBar, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, session, store, session_id, context_limit, model_label, workdir).await;

    disable_raw_mode()?;
    execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_app(
    terminal: &mut Term,
    session: SessionState,
    store: Arc<dyn Store>,
    session_id: String,
    context_limit: u64,
    model_label: String,
    workdir: PathBuf,
) -> Result<()> {
    // Wire a cancellation token into the session so double-Esc can hard-abort
    // the running turn (mid-stream / mid-tool). The UI keeps a clone to signal.
    let cancel = CancellationToken::new();
    let session = session.with_cancel(cancel.clone());

    let mut chat = ChatView { agent: session.agent.name.clone(), ..Default::default() };
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
    let mut steer_count: u32 = 0;
    let mut queue_count: u32 = 0;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut active_skill: Option<String> = None;
    let mut anim_tick: u32 = 0;
    let mut last_esc: Option<Instant> = None;

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(512);

    let worker = tokio::spawn(async move {
        let mut sess = session;
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                UiCmd::Prompt(prompt) => {
                    let tx = evt_tx.clone();
                    let res = run_session(&mut sess, prompt, move |sev| {
                        let _ = tx.try_send(UiEvent::Session(sev));
                    }).await;
                    if let Err(e) = res {
                        let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::Error(format!("{e:#}"))));
                    }
                    let _ = evt_tx.try_send(UiEvent::TurnDone);
                }
                UiCmd::SwitchAgent(name) => {
                    if let Some(a) = resolve_agent(&name) {
                        sess.agent = a;
                        let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::AgentSwitch(name)));
                    }
                }
                UiCmd::SetSkill(body) => {
                    sess.skill_prompt = body;
                }
                UiCmd::Quit => break,
            }
        }
    });

    let mut events = EventStream::new();
    let mut anim_ticker = tokio::time::interval(Duration::from_millis(ANIM_TICK_MS));
    loop {
        let agent_name = chat.agent.clone();
        let status = chat.status.clone();
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
            context_limit,
            &model_label,
            &workdir,
            &status,
            steer_count,
            queue_count,
            scroll,
            follow,
            anim_tick,
            active_skill.as_deref(),
            skill_menu.as_ref(),
            &mut hits,
        )?;

        tokio::select! {
            maybe_evt = events.next() => {
                if let Some(Ok(ev)) = maybe_evt {
                    match ev {
                        Event::Key(k) => {
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
                                        queue_count += 1;
                                        chat.lines.push(Line::from(Span::styled(
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
                                    steer_count += 1;
                                    chat.lines.push(Line::from(Span::styled(
                                        format!("\u{21b3} steer: {text}"), Style::default().fg(Color::Blue))));
                                    follow = true;
                                }
                                KeyAction::Queue(text) => {
                                    let _ = store.admit_input(&mk_input(&session_id, Delivery::Queue, &text)).await;
                                    queue_count += 1;
                                    chat.lines.push(Line::from(Span::styled(
                                        format!("[queued] {text}"), Style::default().fg(Color::Yellow))));
                                    follow = true;
                                }
                                KeyAction::SwitchAgent(name) => {
                                    let _ = cmd_tx.send(UiCmd::SwitchAgent(name)).await;
                                }
                                KeyAction::SetSkill(opt) => {
                                    match opt {
                                        Some((name, body)) => {
                                            active_skill = Some(name.clone());
                                            let _ = cmd_tx.send(UiCmd::SetSkill(Some(body))).await;
                                        }
                                        None => {
                                            active_skill = None;
                                            let _ = cmd_tx.send(UiCmd::SetSkill(None)).await;
                                        }
                                    }
                                }
                                KeyAction::Cancel => {
                                    cancel.cancel();
                                    chat.lines.push(Line::from(Span::styled(
                                        "[interrupted] stopping…", Style::default().fg(Color::Yellow))));
                                    running = false;
                                    follow = true;
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
                                            let max_scroll = chat.lines.len().saturating_sub(visible_h) as u16;
                                            scroll = scroll.saturating_add(3);
                                            if scroll >= max_scroll {
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
                        track_context(&sev, &mut context_used);
                        chat.apply(&sev);
                        if matches!(sev, SessionEvent::Done | SessionEvent::Error(_)) {
                            running = false;
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

fn push_user(chat: &mut ChatView, history: &mut Vec<String>, hist_idx: &mut Option<usize>, text: &str) {
    history.push(text.to_string());
    *hist_idx = None;
    chat.lines.push(Line::from(Span::styled(
        format!("user: {text}"), Style::default().add_modifier(Modifier::BOLD))));
    chat.lines.push(Line::from(""));
}

pub(crate) enum KeyAction {
    None,
    Submit(String),
    Steer(String),
    Queue(String),
    SwitchAgent(String),
    Cancel,
    SetSkill(Option<(String, String)>),
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
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        match k.code {
            KeyCode::Char('c') | KeyCode::Char('d') => return KeyAction::Quit,
            // Toggle by the CURRENT agent so plan<->act works both ways. The
            // previous logic keyed off `running`, which could only ever land
            // on "plan" while idle.
            KeyCode::Char('t') => {
                let next = if agent == "plan" { "act" } else { "plan" };
                return KeyAction::SwitchAgent(next.into());
            }
            KeyCode::Char('h') => { *show_help = !*show_help; return KeyAction::None; }
            KeyCode::Char('n') => { move_hist(history, hist_idx, input, cursor_idx, 1); return KeyAction::None; }
            KeyCode::Char('p') => { move_hist(history, hist_idx, input, cursor_idx, -1); return KeyAction::None; }
            KeyCode::Char('o') => {
                if running && !input.trim().is_empty() {
                    let t = input.trim().to_string();
                    input.clear(); *cursor_idx = 0; *hist_idx = None;
                    return KeyAction::Steer(t);
                }
                return KeyAction::None;
            }
            KeyCode::Char('j') => {
                if running && !input.trim().is_empty() {
                    let t = input.trim().to_string();
                    input.clear(); *cursor_idx = 0; *hist_idx = None;
                    return KeyAction::Queue(t);
                }
                return KeyAction::None;
            }
            KeyCode::Char('u') => { *scroll = scroll.saturating_sub(10); *follow = false; return KeyAction::None; }
            _ => return KeyAction::None,
        }
    }
    match k.code {
        KeyCode::Enter => {
            if input.trim().is_empty() { return KeyAction::None; }
            let text = input.trim().to_string();
            input.clear(); *cursor_idx = 0; *hist_idx = None;
            KeyAction::Submit(text)
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
        KeyCode::Up => { move_hist(history, hist_idx, input, cursor_idx, -1); KeyAction::None }
        KeyCode::Down => { move_hist(history, hist_idx, input, cursor_idx, 1); KeyAction::None }
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
            if c == '$' && input.is_empty() && *cursor_idx == 0 {
                *skill_menu = Some(SkillMenu::new(discover_skills(), active_skill.is_some()));
                return KeyAction::None;
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
struct MouseHits {
    jump_btn: Option<Rect>,
    body: Option<Rect>,
}

fn in_rect(r: Rect, col: u16, row: u16) -> bool {
    col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
}

#[allow(clippy::too_many_arguments)]
fn render(
    terminal: &mut Term,
    chat: &ChatView,
    input: &str,
    cursor_idx: usize,
    agent: &str,
    running: bool,
    show_help: bool,
    context_used: u64,
    context_limit: u64,
    model: &str,
    workdir: &Path,
    status: &str,
    steer_count: u32,
    queue_count: u32,
    scroll: u16,
    follow: bool,
    anim_tick: u32,
    active_skill: Option<&str>,
    skill_menu: Option<&SkillMenu>,
    hits: &mut MouseHits,
) -> Result<()> {
    terminal.draw(|f| {
        let area = f.area();
        // No top header: header info is merged into the bottom status bar.
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),    // body (transcript)
                Constraint::Length(3), // composer
                Constraint::Length(1), // status (merged header + status)
            ])
            .split(area);

        render_body(f, chunks[0], chat, scroll, follow, &mut hits.body);
        render_composer(f, chunks[1], input, follow, &mut hits.jump_btn);
        render_status(
            f,
            chunks[2],
            running,
            status,
            steer_count,
            queue_count,
            model,
            agent,
            workdir,
            context_used,
            context_limit,
            anim_tick,
            active_skill,
        );

        if show_help {
            render_help_popup(f, area);
        }

        if let Some(menu) = skill_menu {
            render_skill_popup(f, area, menu);
        }

        place_cursor(f, chunks[1], input, cursor_idx);
    })?;
    Ok(())
}

fn render_body(f: &mut Frame, area: Rect, chat: &ChatView, scroll: u16, follow: bool, body_out: &mut Option<Rect>) {
    *body_out = Some(area);
    let block = Block::default().borders(Borders::ALL).title(format!(" {} ", chat.agent));
    let inner = block.inner(area);
    let visible_h = inner.height as usize;
    let total = chat.lines.len();
    let max_scroll = total.saturating_sub(visible_h) as u16;
    let pos = if follow { max_scroll } else { scroll.min(max_scroll) };
    let para = chat.render_paragraph(pos).block(block);
    f.render_widget(para, area);

    // Vertical scrollbar so the transcript area is scrollable by position.
    let mut sb_state = ScrollbarState::new(total.max(visible_h))
        .viewport_content_length(visible_h)
        .position(pos as usize);
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        inner,
        &mut sb_state,
    );
}

fn render_composer(
    f: &mut Frame,
    area: Rect,
    input: &str,
    follow: bool,
    jump_btn: &mut Option<Rect>,
) {
    // No hint title on the composer (removed "Enter=send ..." placeholder).
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let line = Line::from(vec![
        Span::styled("\u{276f} ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(input.to_string()),
    ]);
    f.render_widget(Paragraph::new(line).wrap(Wrap { trim: false }), inner);

    // Top-right follow indicator: "跟随中…" when pinned to bottom, else a
    // clickable "↓" button whose rect is exported for mouse hit-testing.
    let (label, style) = if follow {
        ("跟随中…", Style::default().fg(Color::Cyan))
    } else {
        ("↓", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
    };
    let disp_w: u16 = label.chars().map(composer::char_width).sum::<usize>() as u16;
    let lbl_w = disp_w.saturating_add(2).min(area.width);
    let lbl_rect = Rect::new(area.right().saturating_sub(1).saturating_sub(lbl_w), area.y, lbl_w, 1);
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(label, style)])),
        lbl_rect,
    );
    *jump_btn = if follow { None } else { Some(lbl_rect) };
}

#[allow(clippy::too_many_arguments)]
fn render_status(
    f: &mut Frame,
    area: Rect,
    running: bool,
    status: &str,
    steer_count: u32,
    queue_count: u32,
    model: &str,
    agent: &str,
    workdir: &Path,
    used: u64,
    limit: u64,
    anim_tick: u32,
    active_skill: Option<&str>,
) {
    let pct = fmtmod::context_percent(used, limit, CONTEXT_BASELINE);
    let ctx_color = if pct >= 85 { Color::Red } else if pct >= 60 { Color::Yellow } else { Color::Green };
    let dir_name = workdir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| ".".into());

    let mut spans = vec![
        Span::styled(" opencoder ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("| "),
        Span::styled(model.to_string(), Style::default().fg(Color::White)),
        Span::raw(" | "),
        Span::styled(format!("[{agent}]"), Style::default().fg(Color::Magenta)),
        Span::raw(" | "),
        Span::styled(dir_name, Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(
            format!("ctx {}% ({}/{})", pct, fmtmod::format_tokens_compact(used), fmtmod::format_tokens_compact(limit)),
            Style::default().fg(ctx_color),
        ),
    ];

    if let Some(name) = active_skill {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("skill:{name}"),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ));
    }

    // Running state is shown as an animated spinner (no static "idle" word).
    if running {
        let spin = SPINNER[(anim_tick as usize) % SPINNER.len()];
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format!("{spin} {status}"), Style::default().fg(Color::Yellow)));
    } else if !status.is_empty() {
        spans.push(Span::styled(format!("  | {status}"), Style::default().fg(Color::DarkGray)));
    }
    if steer_count > 0 {
        spans.push(Span::styled(format!(" | \u{21b3}steer:{steer_count}"), Style::default().fg(Color::Blue)));
    }
    if queue_count > 0 {
        spans.push(Span::styled(format!(" | queue:{queue_count}"), Style::default().fg(Color::Yellow)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_help_popup(f: &mut Frame, area: Rect) {
    let h = 20u16.min(area.height.saturating_sub(2));
    let w = 60u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL).title(" Help (Ctrl+H, Esc to close) ");
    f.render_widget(Paragraph::new(crate::keybind::HELP).block(block), popup);
}

fn place_cursor(f: &mut Frame, composer_area: Rect, input: &str, cursor_idx: usize) {
    let border = 1u16;
    let prompt_w = 2u16; // "❯ "
    let col = composer::cursor_column(input, cursor_idx);
    let x = composer_area.x + border + prompt_w + col;
    let y = composer_area.y + border;
    f.set_cursor_position((x, y));
}

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
