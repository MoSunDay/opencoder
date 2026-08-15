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
        "/config",
        "配置模型 / 思考深度 / base_url / api_key / 上下文阈值 / 渲染帧率 / tmux",
    ),
    (
        "/compact",
        "手动压缩对话历史（总结早期消息，释放上下文窗口）",
    ),
    ("/act", "切换到 act 模式（不重置上下文）"),
    ("/plan", "切换到 plan 模式（不重置上下文）"),
    ("/annotation", "记录/编辑任务备注 (annotation editor)"),
    ("/notepad", "IDE 式文件浏览/编辑 (文件树 + vim 编辑器)"),
    (
        "/act_clear_context",
        "清空对话上下文并切换到 act 模式（重新开始）",
    ),
    ("/ps", "查看所有后台 bash 进程（不计入模型上下文）"),
    ("/stop", "强制结束所有后台 bash 进程（不计入模型上下文）"),
    ("/ap", "切换 autopilot 自动模式（不计入模型上下文）"),
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
    Annotation,
    Notepad,
    ClearContext,
    /// `/mcp` — manage MCP servers (enable/disable/add/edit/delete).
    Mcp,
    /// `/cli` — manage CLI prompt registrations.
    Cli,
    /// Display-only: list background bash (never enters model context).
    Ps,
    /// Display-only: kill all background bash (never enters model context).
    Stop,
    /// Display-only: toggle autopilot (never enters model context).
    Ap,
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
        self.rows = COMMANDS
            .iter()
            .enumerate()
            .filter(|(_, (name, desc))| {
                if q.is_empty() {
                    return true;
                }
                let name_l = name.trim_start_matches('/').to_lowercase();
                name_l.contains(q) || desc.to_lowercase().contains(q)
            })
            .map(|(i, _)| i)
            .collect();
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
        "annotation" | "ann" => Some(SlashAction::Annotation),
        "notepad" | "note" => Some(SlashAction::Notepad),
        "act_clear_context" => Some(SlashAction::ClearContext),
        "mcp" | "mc" => Some(SlashAction::Mcp),
        "cli" => Some(SlashAction::Cli),
        "ps" => Some(SlashAction::Ps),
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
        "/annotation" => Some(SlashAction::Annotation),
        "/notepad" => Some(SlashAction::Notepad),
        "/act_clear_context" => Some(SlashAction::ClearContext),
        "/mcp" => Some(SlashAction::Mcp),
        "/cli" => Some(SlashAction::Cli),
        "/ps" => Some(SlashAction::Ps),
        "/stop" => Some(SlashAction::Stop),
        "/ap" => Some(SlashAction::Ap),
        _ => None,
    }
}

/// Map a [`SlashAction`] to its canonical control-command string, or `None`
/// for non-control actions. Used to queue a control command (Tab) or dispatch
/// it immediately (Enter) without echoing it as user text.
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
        // natural compound input such as `/plan <requirement>` works.
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
mod tests {
    use super::*;

    #[test]
    fn parse_known_commands() {
        assert_eq!(parse("/config"), Some(SlashAction::Config));
        assert_eq!(parse("/cfg"), Some(SlashAction::Config));
        assert_eq!(parse("/task"), Some(SlashAction::Task));
        assert_eq!(parse("/t"), Some(SlashAction::Task));
        assert_eq!(parse("/compact"), Some(SlashAction::Compact));
        assert_eq!(parse("/c"), Some(SlashAction::Compact));
        assert_eq!(parse("/"), Some(SlashAction::Task));
        assert_eq!(parse("/unknown"), None);
        assert_eq!(parse("hello"), None);
        assert_eq!(parse(" /config "), Some(SlashAction::Config));
    }

    #[test]
    fn menu_filters_by_query() {
        let mut m = CommandMenu::new();
        assert!(
            m.visible_count() >= 3,
            "all commands visible with empty query"
        );
        for c in "config".chars() {
            m.on_char(c);
        }
        assert_eq!(m.visible_count(), 1, "only /config matches 'config'");
        assert_eq!(m.selected_action(), Some(SlashAction::Config));
    }

    #[test]
    fn menu_filters_compact() {
        let mut m = CommandMenu::new();
        for c in "compact".chars() {
            m.on_char(c);
        }
        assert_eq!(m.visible_count(), 1, "only /compact matches 'compact'");
        assert_eq!(m.selected_action(), Some(SlashAction::Compact));
    }

    #[test]
    fn empty_query_defaults_to_task() {
        let m = CommandMenu::new();
        assert_eq!(
            m.selected_action(),
            Some(SlashAction::Task),
            "first row is /task"
        );
    }

    #[test]
    fn paste_appends_to_query_and_refilters() {
        let mut m = CommandMenu::new();
        let all = m.visible_count();
        assert!(m.query().is_empty());
        m.paste("task");
        assert_eq!(m.query(), "task");
        assert!(m.visible_count() >= 1, "filter should still match 'task'");
        assert!(
            m.visible_count() < all,
            "refilter should narrow the visible list"
        );
    }

    #[test]
    fn parse_control_commands() {
        assert_eq!(parse("/act"), Some(SlashAction::Act));
        assert_eq!(parse("/plan"), Some(SlashAction::Plan));
        assert_eq!(parse("/act_clear_context"), Some(SlashAction::ClearContext));
        assert_eq!(parse(" /plan "), Some(SlashAction::Plan));
    }

    #[test]
    fn control_cmd_string_maps_correctly() {
        assert_eq!(control_cmd_string(&SlashAction::Act), Some("/act"));
        assert_eq!(control_cmd_string(&SlashAction::Plan), Some("/plan"));
        assert_eq!(
            control_cmd_string(&SlashAction::ClearContext),
            Some("/act_clear_context")
        );
        assert_eq!(control_cmd_string(&SlashAction::Task), None);
        assert_eq!(control_cmd_string(&SlashAction::Compact), None);
        assert_eq!(control_cmd_string(&SlashAction::Ps), None);
        assert_eq!(control_cmd_string(&SlashAction::Stop), None);
    }

    #[test]
    fn tab_fills_input_with_command_name() {
        let mut menu = Some(CommandMenu::new());
        // Filter to /plan
        for c in "plan".chars() {
            if let Some(m) = menu.as_mut() {
                m.on_char(c);
            }
        }
        let (outcome, _quit) = handle_command_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        match outcome {
            CommandOutcome::FillInput(s) => assert_eq!(s, "/plan"),
            other => panic!("expected FillInput, got {:?}", other),
        }
        assert!(menu.is_none(), "popup closed after Tab-fill");
    }

    #[test]
    fn space_fills_selected_command_for_compound_input() {
        let mut menu = Some(CommandMenu::new());
        for c in "plan".chars() {
            menu.as_mut().expect("menu open").on_char(c);
        }

        let (outcome, quit) =
            handle_command_key(&mut menu, key(KeyCode::Char(' '), KeyModifiers::NONE));

        assert!(!quit);
        assert!(matches!(outcome, CommandOutcome::FillInput(ref s) if s == "/plan"));
        assert!(menu.is_none(), "popup must close after Space-fill");
    }

    #[test]
    fn space_with_no_matching_command_keeps_popup_open() {
        let mut menu = Some(CommandMenu::new());
        menu.as_mut().expect("menu open").paste("no-such-command");

        let (outcome, quit) =
            handle_command_key(&mut menu, key(KeyCode::Char(' '), KeyModifiers::NONE));

        assert!(!quit);
        assert!(matches!(outcome, CommandOutcome::Idle));
        assert_eq!(
            menu.as_ref().expect("popup stays open").query(),
            "no-such-command"
        );
    }

    #[test]
    fn tab_on_non_control_command_fills_input() {
        let mut menu = Some(CommandMenu::new());
        // Filter to /task (non-control)
        for c in "task".chars() {
            if let Some(m) = menu.as_mut() {
                m.on_char(c);
            }
        }
        let (outcome, _quit) = handle_command_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        match outcome {
            CommandOutcome::FillInput(s) => assert_eq!(s, "/task"),
            other => panic!("expected FillInput, got {:?}", other),
        }
        assert!(menu.is_none(), "popup closed after Tab-fill");
    }

    #[test]
    fn enter_on_control_command_dispatches() {
        let mut menu = Some(CommandMenu::new());
        // Type "act" — matches /compact, /act, /act_clear_context.
        for c in "act".chars() {
            if let Some(m) = menu.as_mut() {
                m.on_char(c);
            }
        }
        // Move down to /act (index 1 after /compact).
        if let Some(m) = menu.as_mut() {
            m.move_down();
        }
        let (outcome, _quit) =
            handle_command_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        match outcome {
            CommandOutcome::Dispatch(SlashAction::Act) => {}
            other => panic!("expected Dispatch(Act), got {:?}", other),
        }
        assert!(menu.is_none(), "popup closed after Enter-dispatch");
    }

    #[test]
    fn enter_on_clear_context_dispatches() {
        let mut menu = Some(CommandMenu::new());
        for c in "act_clear_context".chars() {
            if let Some(m) = menu.as_mut() {
                m.on_char(c);
            }
        }
        let (outcome, _quit) =
            handle_command_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        match outcome {
            CommandOutcome::Dispatch(SlashAction::ClearContext) => {}
            other => panic!("expected Dispatch(ClearContext), got {:?}", other),
        }
    }

    #[test]
    fn parse_local_commands() {
        assert_eq!(parse("/ps"), Some(SlashAction::Ps));
        assert_eq!(parse("/stop"), Some(SlashAction::Stop));
        assert_eq!(parse("/ap"), Some(SlashAction::Ap));
        assert_eq!(parse(" /ps "), Some(SlashAction::Ps));
        assert_eq!(parse(" /ap "), Some(SlashAction::Ap));
    }

    #[test]
    fn enter_on_ps_dispatches() {
        let mut menu = Some(CommandMenu::new());
        for c in "ps".chars() {
            if let Some(m) = menu.as_mut() {
                m.on_char(c);
            }
        }
        let (outcome, _quit) =
            handle_command_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        match outcome {
            CommandOutcome::Dispatch(SlashAction::Ps) => {}
            other => panic!("expected Dispatch(Ps), got {:?}", other),
        }
        assert!(menu.is_none(), "popup closed after Enter-dispatch");
    }

    #[test]
    fn enter_on_stop_dispatches() {
        let mut menu = Some(CommandMenu::new());
        for c in "stop".chars() {
            if let Some(m) = menu.as_mut() {
                m.on_char(c);
            }
        }
        let (outcome, _quit) =
            handle_command_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        match outcome {
            CommandOutcome::Dispatch(SlashAction::Stop) => {}
            other => panic!("expected Dispatch(Stop), got {:?}", other),
        }
    }

    #[test]
    fn enter_on_ap_dispatches() {
        let mut menu = Some(CommandMenu::new());
        for c in "ap".chars() {
            if let Some(m) = menu.as_mut() {
                m.on_char(c);
            }
        }
        // Query "ap" also matches "/config" (its description contains
        // "api_key"), which sorts before "/ap" — move down once to it.
        menu.as_mut().expect("menu open").move_down();
        let (outcome, _quit) =
            handle_command_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        match outcome {
            CommandOutcome::Dispatch(SlashAction::Ap) => {}
            other => panic!("expected Dispatch(Ap), got {:?}", other),
        }
        assert!(menu.is_none(), "popup closed after Enter-dispatch");
    }

    #[test]
    fn tab_on_local_command_fills_input() {
        let mut menu = Some(CommandMenu::new());
        for c in "ps".chars() {
            if let Some(m) = menu.as_mut() {
                m.on_char(c);
            }
        }
        let (outcome, _quit) = handle_command_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        match outcome {
            CommandOutcome::FillInput(s) => assert_eq!(s, "/ps"),
            other => panic!("expected FillInput, got {:?}", other),
        }
        assert!(menu.is_none(), "popup closed after Tab-fill");
    }

    #[test]
    fn parse_fork() {
        assert_eq!(parse("/fork"), Some(SlashAction::Fork));
        assert_eq!(parse("/fk"), Some(SlashAction::Fork)); // alias
        assert_eq!(parse("fork"), None); // bare name (no slash) -> None
        assert_eq!(parse(" /fork "), Some(SlashAction::Fork)); // trimmed
    }

    #[test]
    fn dispatch_fork() {
        assert_eq!(dispatch("/fork"), Some(SlashAction::Fork));
        assert_eq!(dispatch("/fk"), None); // alias resolved by parse, not dispatch
    }

    #[test]
    fn enter_on_fork_dispatches() {
        let mut menu = Some(CommandMenu::new());
        for c in "fork".chars() {
            if let Some(m) = menu.as_mut() {
                m.on_char(c);
            }
        }
        let (outcome, _quit) =
            handle_command_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        match outcome {
            CommandOutcome::Dispatch(SlashAction::Fork) => {}
            other => panic!("expected Dispatch(Fork), got {:?}", other),
        }
        assert!(menu.is_none(), "popup closed after Enter-dispatch");
    }

    #[test]
    fn short_key_command_removed() {
        assert_eq!(parse("/short_key"), None);
        assert_eq!(parse("/sk"), None);
        assert_eq!(parse("short_key"), None);
        assert_eq!(dispatch("/short_key"), None);
    }

    #[test]
    fn parse_annotation_full() {
        assert_eq!(parse("/annotation"), Some(SlashAction::Annotation));
    }

    #[test]
    fn parse_annotation_alias() {
        assert_eq!(parse("/ann"), Some(SlashAction::Annotation));
    }

    #[test]
    fn dispatch_annotation() {
        assert_eq!(dispatch("/annotation"), Some(SlashAction::Annotation));
    }

    #[test]
    fn parse_notepad_full() {
        assert_eq!(parse("/notepad"), Some(SlashAction::Notepad));
    }

    #[test]
    fn parse_notepad_alias() {
        assert_eq!(parse("/note"), Some(SlashAction::Notepad));
    }

    #[test]
    fn dispatch_notepad() {
        assert_eq!(dispatch("/notepad"), Some(SlashAction::Notepad));
    }

    #[test]
    fn parse_mcp_full() {
        assert_eq!(parse("/mcp"), Some(SlashAction::Mcp));
    }

    #[test]
    fn parse_mcp_alias() {
        assert_eq!(parse("/mc"), Some(SlashAction::Mcp));
    }

    #[test]
    fn parse_model_and_alias() {
        assert_eq!(parse("/model"), Some(SlashAction::Model));
        assert_eq!(parse("/mdl"), Some(SlashAction::Model));
    }

    #[test]
    fn dispatch_mcp() {
        assert_eq!(dispatch("/mcp"), Some(SlashAction::Mcp));
    }

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }
}
