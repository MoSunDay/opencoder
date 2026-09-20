//! Slash-command registry + picker popup (`/`) for the TUI composer.
//!
//! Typing `/` as the first character opens [`CommandMenu`]: a centered overlay
//! listing the registered slash commands, filtered live by what follows the
//! slash. `Enter` dispatches the highlighted command (returned as a
//! [`SlashAction`]); `Esc` cancels. Mirrors the skill-menu (`$`) structure so
//! `app.rs` stays a flat match.
//!
//! This is the single source of truth for slash commands: add an entry to
//! [`COMMANDS`] and a branch to [`parse`] / [`CommandMenu::dispatch`] to teach
//! the TUI a new `/xxx` command.

use crate::theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

/// Registered slash commands: `(invocation, description)`. The first entry is
/// the default highlight when the popup opens with an empty query.
pub const COMMANDS: &[(&str, &str)] = &[
    ("/task", "切换 / 新建 / 恢复会话 (task picker)"),
    ("/fork", "从已有会话复制上下文创建新任务 (fork picker)"),
    ("/model", "切换供应商 / 模型 (provider picker)"),
    ("/mcp", "管理 MCP server 列表 (enable/disable/增删改)"),
    ("/cli", "管理 CLI 注册内容及注入范围 (parent/subagents/all)"),
    (
        "/skill",
        "管理默认注入的 skill (ON=目录+名称+概要 注入 context 尾部)",
    ),
    (
        "/config",
        "配置模型 / 思考深度 / base_url / api_key / 上下文阈值 / 渲染帧率 / tmux",
    ),
    (
        "/compact",
        "手动压缩对话历史（总结早期消息，释放上下文窗口）",
    ),
    ("/act", "退出只读模式，切换到 act 执行代理（不重置上下文）"),
    (
        "/plan",
        "只读探索：拦截写操作，切换到 plan 探索代理（不重置上下文）",
    ),
    (
        "/agent",
        "切换 primary agent（回车打开 agent 选择器，或 /agent <name> 直接切换）",
    ),
    ("/annotation", "记录/编辑任务备注 (annotation editor)"),
    ("/notepad", "IDE 式文件浏览/编辑 (文件树 + vim 编辑器)"),
    (
        "/act_clear_context",
        "清空对话上下文并执行（plan 下保留计划并切到 act；/clear_context 同效）",
    ),
    ("/ps", "查看所有后台 bash 进程（不计入模型上下文）"),
    ("/stop", "强制结束所有后台 bash 进程（不计入模型上下文）"),
    (
        "/sidecar",
        "旁路快照问答：进入临时问询界面，ESC 返回即销毁不留痕（token 计入主任务）",
    ),
    ("/ap", "选择 autopilot 模式 (off / 完全自动 / 自动 review)"),
];

/// Action produced by dispatching a slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashAction {
    Task,
    Fork,
    Model,
    Config,
    Compact,
    CacheSalt,
    Act,
    Plan,
    /// `/agent` — open the primary-agent picker (a pick fills the composer
    /// with `/agent <name> `; a manually typed name rides the prompt path).
    Agent,
    Annotation,
    Notepad,
    ClearContext,
    /// `/mcp` — manage MCP servers (enable/disable/add/edit/delete).
    Mcp,
    /// `/cli` — manage CLI prompt registrations.
    Cli,
    /// `/skill` — manage default-injection skill toggles.
    Skill,
    /// Display-only: list background bash (never enters model context).
    Ps,
    /// Display-only: kill all background bash (never enters model context).
    Stop,
    /// Display-only: open the autopilot mode menu (never enters model
    /// context).
    Ap,
    /// `/sidecar` — enter the bypass Q/A panel (never enters model context;
    /// destroy-on-entry / destroy-on-exit).
    Sidecar,
}

/// Outcome of a keystroke while the command popup is open. `Dispatch` carries
/// the chosen action and closes the popup; `Idle` leaves it open.
#[derive(Debug)]
pub enum CommandOutcome {
    Idle,
    Dispatch(SlashAction),
    /// Fill the main input with the selected command name and close the popup
    /// (Tab in the popup). The user can then edit and submit from the composer.
    FillInput(String),
}

/// Picker state for the `/` command menu.
#[derive(Default)]
pub struct CommandMenu {
    /// Filtered rows (indices into [`COMMANDS`]).
    rows: Vec<usize>,
    selected: usize,
    query: String,
}

impl CommandMenu {
    pub fn new() -> Self {
        let mut m = Self::default();
        m.refilter();
        m
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn visible_count(&self) -> usize {
        self.rows.len()
    }

    pub fn move_up(&mut self) {
        let n = self.visible_count();
        if n > 0 {
            self.selected = (self.selected + n - 1) % n;
        }
    }

    pub fn move_down(&mut self) {
        let n = self.visible_count();
        if n > 0 {
            self.selected = (self.selected + 1) % n;
        }
    }

    pub fn on_char(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    /// Paste multi-char text into the query and refilter (mirrors `on_char`).
    pub fn paste(&mut self, text: &str) {
        self.query.push_str(text);
        self.refilter();
    }

    pub fn on_backspace(&mut self) {
        self.query.pop();
        self.refilter();
    }

    /// Resolve the highlighted row to an action, if any.
    pub fn selected_action(&self) -> Option<SlashAction> {
        let idx = *self.rows.get(self.selected)?;
        dispatch(COMMANDS[idx].0)
    }

    /// Resolve the highlighted row to its invocation name (e.g. "/config").
    pub fn selected_name(&self) -> Option<&'static str> {
        let idx = *self.rows.get(self.selected)?;
        Some(COMMANDS[idx].0)
    }

    fn refilter(&mut self) {
        let q = self.query.trim().to_lowercase();
        let q = q.strip_prefix('/').unwrap_or(&q);
        // A complete command must win over a substring such as /compact
        // matching "act". Prefixes then substrings precede description hits;
        // ties keep registration order, including the empty-query default.
        let mut ranked = COMMANDS
            .iter()
            .enumerate()
            .filter_map(|(i, (name, desc))| {
                let rank = if q.is_empty() {
                    0
                } else {
                    let name_l = name.trim_start_matches('/').to_lowercase();
                    if name_l == q {
                        0
                    } else if name_l.starts_with(q) {
                        1
                    } else if name_l.contains(q) {
                        2
                    } else if desc.to_lowercase().contains(q) {
                        3
                    } else {
                        return None;
                    }
                };
                Some((i, rank))
            })
            .collect::<Vec<_>>();
        ranked.sort_by_key(|&(_, rank)| rank);
        self.rows = ranked.into_iter().map(|(i, _)| i).collect();
        self.selected = if self.rows.is_empty() {
            0
        } else {
            self.selected.min(self.rows.len() - 1)
        };
    }
}

/// Map a committed command string (with or without leading `/`) to an action.
/// Used both by the popup's `Enter` and by free-text parse on the composer
/// (so `/config<Enter>` works even without ever opening the popup).
pub fn parse(input: &str) -> Option<SlashAction> {
    let t = input.trim();
    let bare = t.strip_prefix('/')?;
    match bare {
        "" | "t" | "task" => Some(SlashAction::Task),
        "fork" | "fk" => Some(SlashAction::Fork),
        "model" | "mdl" => Some(SlashAction::Model),
        "config" | "cfg" => Some(SlashAction::Config),
        "c" | "compact" => Some(SlashAction::Compact),
        "act" => Some(SlashAction::Act),
        "plan" => Some(SlashAction::Plan),
        "agent" => Some(SlashAction::Agent),
        "annotation" | "ann" => Some(SlashAction::Annotation),
        "notepad" | "note" => Some(SlashAction::Notepad),
        "act_clear_context" | "clear_context" => Some(SlashAction::ClearContext),
        "mcp" | "mc" => Some(SlashAction::Mcp),
        "cli" => Some(SlashAction::Cli),
        "skill" | "sk" => Some(SlashAction::Skill),
        "ps" => Some(SlashAction::Ps),
        "sidecar" => Some(SlashAction::Sidecar),
        "stop" => Some(SlashAction::Stop),
        "ap" => Some(SlashAction::Ap),
        _ => None,
    }
}

fn dispatch(name: &str) -> Option<SlashAction> {
    match name {
        "/task" => Some(SlashAction::Task),
        "/fork" => Some(SlashAction::Fork),
        "/model" => Some(SlashAction::Model),
        "/config" => Some(SlashAction::Config),
        "/compact" => Some(SlashAction::Compact),
        "/act" => Some(SlashAction::Act),
        "/plan" => Some(SlashAction::Plan),
        "/agent" => Some(SlashAction::Agent),
        "/annotation" => Some(SlashAction::Annotation),
        "/notepad" => Some(SlashAction::Notepad),
        "/act_clear_context" | "/clear_context" => Some(SlashAction::ClearContext),
        "/mcp" => Some(SlashAction::Mcp),
        "/cli" => Some(SlashAction::Cli),
        "/skill" => Some(SlashAction::Skill),
        "/ps" => Some(SlashAction::Ps),
        "/stop" => Some(SlashAction::Stop),
        "/sidecar" => Some(SlashAction::Sidecar),
        "/ap" => Some(SlashAction::Ap),
        _ => None,
    }
}

/// Map a [`SlashAction`] to its canonical control-command string, or `None`
/// for non-control actions. Used to queue a control command (Tab) or dispatch
/// it immediately (Enter) without echoing it as user text. The legacy
/// `/clear_context` spelling still parses as an alias of
/// `/act_clear_context`.
pub fn control_cmd_string(action: &SlashAction) -> Option<&'static str> {
    match action {
        SlashAction::Act => Some("/act"),
        SlashAction::Plan => Some("/plan"),
        SlashAction::ClearContext => Some("/act_clear_context"),
        _ => None,
    }
}

/// Handle one keystroke against an open command menu. When the menu is closed
/// (Esc, or a dispatch) the `Option` is set to `None` so the caller drops modal
/// mode. `Ctrl+D` propagates as `None` (caller decides quit).
pub fn handle_command_key(menu: &mut Option<CommandMenu>, k: KeyEvent) -> (CommandOutcome, bool) {
    let m = match menu.as_mut() {
        Some(m) => m,
        None => return (CommandOutcome::Idle, false),
    };
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(k.code, KeyCode::Char('d') | KeyCode::Char('\u{4}')) {
            let quit = true;
            *menu = None;
            return (CommandOutcome::Idle, quit);
        }
        return (CommandOutcome::Idle, false);
    }
    let outcome = match k.code {
        KeyCode::Up => {
            m.move_up();
            CommandOutcome::Idle
        }
        KeyCode::Down => {
            m.move_down();
            CommandOutcome::Idle
        }
        KeyCode::Backspace => {
            m.on_backspace();
            if m.query().is_empty() {
                // Empty query — keep the menu open showing all commands.
            }
            CommandOutcome::Idle
        }
        // A command token cannot contain spaces. Complete the highlighted
        // command before requirement text reaches the filter query, so
        // natural compound input such as `/plan <topic>` works.
        KeyCode::Char(' ') if k.modifiers.is_empty() => match m.selected_name() {
            Some(name) => {
                let name = name.to_string();
                *menu = None;
                CommandOutcome::FillInput(name)
            }
            None => CommandOutcome::Idle,
        },
        KeyCode::Char(c) => {
            m.on_char(c);
            CommandOutcome::Idle
        }
        KeyCode::Enter => match m.selected_action() {
            Some(act) => {
                *menu = None;
                CommandOutcome::Dispatch(act)
            }
            None => CommandOutcome::Idle,
        },
        // Tab fills the input with the highlighted command name and closes the
        // popup. The user can then edit and submit (Enter) from the composer.
        KeyCode::Tab => match m.selected_name() {
            Some(name) => {
                *menu = None;
                CommandOutcome::FillInput(name.to_string())
            }
            None => CommandOutcome::Idle,
        },
        KeyCode::Esc => {
            *menu = None;
            CommandOutcome::Idle
        }
        _ => CommandOutcome::Idle,
    };
    (outcome, false)
}

/// Draw the command menu as a dropdown overlay anchored above the composer.
///
/// `composer_top` is the screen row of the composer's top border; the popup's
/// bottom edge (plus its 1-row query footer) sits just above it, mimicking an
/// IDE autocomplete dropdown rather than a centered modal.
pub fn render_command_popup(f: &mut Frame, area: Rect, composer_top: u16, menu: &CommandMenu) {
    // Box = 2 borders + content rows; +1 row for the query footer drawn below.
    let want_box = menu.visible_count() as u16 + 4;
    let want_total = want_box.saturating_add(1);
    let avail = composer_top.max(1);
    let total = want_total.min(avail);
    let h = total.saturating_sub(1).max(3);
    let w = 72u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(total);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let block = crate::theme::rounded_block(
        "/commands (\u{2191}/\u{2193} move, type to filter, Space/Tab=fill, Enter=confirm, Esc=cancel)",
    );

    let items: Vec<ListItem> = menu
        .rows
        .iter()
        .map(|&i| {
            let (name, desc) = COMMANDS[i];
            ListItem::new(Line::from(vec![
                Span::styled(
                    name.to_string(),
                    Style::default()
                        .fg(theme::accent())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" \u{2014} "),
                Span::styled(desc.to_string(), Style::default().fg(theme::subtle())),
            ]))
        })
        .collect();

    let items = if items.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  no matching command",
            Style::default().fg(theme::muted()),
        )))]
    } else {
        items
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(crate::theme::list_highlight())
        .highlight_symbol("\u{276f} ");

    let mut state = ListState::default();
    if menu.visible_count() > 0 {
        state.select(Some(menu.selected));
    }
    f.render_stateful_widget(list, popup, &mut state);

    // Query footer.
    let footer = Rect::new(
        popup.x,
        popup.bottom(),
        popup.width,
        1u16.min(area.height.saturating_sub(popup.bottom())),
    );
    if footer.height > 0 {
        let line = Line::from(vec![
            Span::styled(" /", Style::default().fg(theme::muted())),
            Span::styled(
                menu.query().to_string(),
                Style::default()
                    .fg(theme::warn_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("_"),
        ]);
        f.render_widget(Paragraph::new(line).wrap(Wrap { trim: false }), footer);
    }
}

#[cfg(test)]
#[path = "command/catalog_tests.rs"]
mod catalog_tests;
#[cfg(test)]
#[path = "command/key_tests.rs"]
mod key_tests;
