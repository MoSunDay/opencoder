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

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

/// Registered slash commands: `(invocation, description)`. The first entry is
/// the default highlight when the popup opens with an empty query.
pub const COMMANDS: &[(&str, &str)] = &[
    ("/task", "切换 / 新建 / 恢复会话 (task picker)"),
    (
        "/model",
        "配置模型 / 思考深度 / base_url / api_key / 上下文阈值",
    ),
    (
        "/compact",
        "手动压缩对话历史（总结早期消息，释放上下文窗口）",
    ),
];

/// Action produced by dispatching a slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashAction {
    Task,
    Model,
    Compact,
}

/// Outcome of a keystroke while the command popup is open. `Dispatch` carries
/// the chosen action and closes the popup; `Idle` leaves it open.
pub enum CommandOutcome {
    Idle,
    Dispatch(SlashAction),
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

    pub fn on_backspace(&mut self) {
        self.query.pop();
        self.refilter();
    }

    /// Resolve the highlighted row to an action, if any.
    pub fn selected_action(&self) -> Option<SlashAction> {
        let idx = *self.rows.get(self.selected)?;
        dispatch(COMMANDS[idx].0)
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
/// (so `/model<Enter>` works even without ever opening the popup).
pub fn parse(input: &str) -> Option<SlashAction> {
    let t = input.trim();
    let bare = t.strip_prefix('/')?;
    match bare {
        "" | "t" | "task" => Some(SlashAction::Task),
        "m" | "model" => Some(SlashAction::Model),
        "c" | "compact" => Some(SlashAction::Compact),
        _ => None,
    }
}

fn dispatch(name: &str) -> Option<SlashAction> {
    match name {
        "/task" => Some(SlashAction::Task),
        "/model" => Some(SlashAction::Model),
        "/compact" => Some(SlashAction::Compact),
        _ => None,
    }
}

/// Handle one keystroke against an open command menu. When the menu is closed
/// (Esc, or a dispatch) the `Option` is set to `None` so the caller drops modal
/// mode. `Ctrl+C` / `Ctrl+D` propagate as `None` (caller decides quit).
pub fn handle_command_key(menu: &mut Option<CommandMenu>, k: KeyEvent) -> (CommandOutcome, bool) {
    let m = match menu.as_mut() {
        Some(m) => m,
        None => return (CommandOutcome::Idle, false),
    };
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(
            k.code,
            KeyCode::Char('c')
                | KeyCode::Char('d')
                | KeyCode::Char('\u{3}')
                | KeyCode::Char('\u{4}')
        ) {
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

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" /commands (\u{2191}/\u{2193} move, type to filter, Enter=confirm, Esc=cancel) ");

    let items: Vec<ListItem> = menu
        .rows
        .iter()
        .map(|&i| {
            let (name, desc) = COMMANDS[i];
            ListItem::new(Line::from(vec![
                Span::styled(
                    name.to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" \u{2014} "),
                Span::styled(desc.to_string(), Style::default().fg(Color::Gray)),
            ]))
        })
        .collect();

    let items = if items.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  no matching command",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        items
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
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
            Span::styled(" /", Style::default().fg(Color::DarkGray)),
            Span::styled(
                menu.query().to_string(),
                Style::default()
                    .fg(Color::Yellow)
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
        assert_eq!(parse("/model"), Some(SlashAction::Model));
        assert_eq!(parse("/m"), Some(SlashAction::Model));
        assert_eq!(parse("/task"), Some(SlashAction::Task));
        assert_eq!(parse("/t"), Some(SlashAction::Task));
        assert_eq!(parse("/compact"), Some(SlashAction::Compact));
        assert_eq!(parse("/c"), Some(SlashAction::Compact));
        assert_eq!(parse("/"), Some(SlashAction::Task));
        assert_eq!(parse("/unknown"), None);
        assert_eq!(parse("hello"), None);
        assert_eq!(parse(" /model "), Some(SlashAction::Model));
    }

    #[test]
    fn menu_filters_by_query() {
        let mut m = CommandMenu::new();
        assert!(
            m.visible_count() >= 3,
            "all commands visible with empty query"
        );
        for c in "model".chars() {
            m.on_char(c);
        }
        assert_eq!(m.visible_count(), 1, "only /model matches 'model'");
        assert_eq!(m.selected_action(), Some(SlashAction::Model));
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
}
