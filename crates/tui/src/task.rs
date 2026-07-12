//! `/task` session picker — switch between or create new conversations.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencode_store::SessionListItem;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState};
use ratatui::Frame;

/// What the user picked from the task picker.
#[derive(Clone, Debug)]
pub enum TaskPick {
    New,
    Resume(String),
}

#[derive(Debug)]
pub enum TaskOutcome {
    Idle,
    Quit,
    Pick(TaskPick),
    /// User confirmed the "Clear all" destructive action. `keep_session_id` is
    /// the currently-active session, which must be preserved.
    ClearAll {
        keep_session_id: String,
    },
}

/// Modal session picker shown when the user types `/task`.
pub struct TaskPicker {
    sessions: Vec<SessionListItem>,
    selected: usize,
    /// The currently-active session id — always preserved by "Clear all", and
    /// tagged `(current)` in the rendered list.
    current_session_id: String,
    /// Two-step confirmation guard for the destructive "Clear all" row.
    /// `true` while we're waiting for the second Enter (or an Esc to cancel).
    confirm_clear: bool,
}

impl TaskPicker {
    pub fn new(sessions: Vec<SessionListItem>, current_session_id: String) -> Self {
        TaskPicker {
            sessions,
            selected: 0,
            current_session_id,
            confirm_clear: false,
        }
    }

    /// Replace the listed sessions (e.g. after a clear) and reset selection +
    /// confirmation state. Used by the app layer to refresh in place.
    pub fn reset_sessions(&mut self, sessions: Vec<SessionListItem>) {
        self.sessions = sessions;
        self.selected = 0;
        self.confirm_clear = false;
    }

    /// Drop the two-step confirmation guard without touching the list. Used
    /// when the destructive op fails and we want to leave the picker usable.
    pub fn reset_confirmation(&mut self) {
        self.confirm_clear = false;
    }

    pub fn deletable_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|s| s.id != self.current_session_id)
            .count()
    }

    /// Index of the "Clear all" row, or `None` when there is nothing deletable
    /// to clear (only the current session, or empty).
    fn clear_row_index(&self) -> Option<usize> {
        if self.deletable_count() == 0 {
            return None;
        }
        Some(1 + self.sessions.len())
    }

    fn row_count(&self) -> usize {
        let base = 1 + self.sessions.len(); // "+ New task" + sessions
        if self.clear_row_index().is_some() {
            base + 1
        } else {
            base
        }
    }

    pub fn move_up(&mut self) {
        let n = self.row_count();
        if n > 0 {
            self.selected = (self.selected + n - 1) % n;
        }
    }

    pub fn move_down(&mut self) {
        let n = self.row_count();
        if n > 0 {
            self.selected = (self.selected + 1) % n;
        }
    }

    pub fn selection(&self) -> Option<TaskPick> {
        if self.selected == 0 {
            Some(TaskPick::New)
        } else {
            self.sessions
                .get(self.selected - 1)
                .map(|s| TaskPick::Resume(s.id.clone()))
        }
    }
}

/// Handle a keystroke in the task picker.
pub fn handle_task_key(picker: &mut Option<TaskPicker>, k: KeyEvent) -> TaskOutcome {
    let p = match picker.as_mut() {
        Some(p) => p,
        None => return TaskOutcome::Idle,
    };
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        match k.code {
            KeyCode::Char('c')
            | KeyCode::Char('d')
            | KeyCode::Char('\u{3}')
            | KeyCode::Char('\u{4}') => return TaskOutcome::Quit,
            _ => return TaskOutcome::Idle,
        }
    }
    match k.code {
        KeyCode::Enter => {
            // Second Enter while the confirmation guard is armed commits the clear.
            if p.confirm_clear {
                let keep = p.current_session_id.clone();
                p.confirm_clear = false;
                return TaskOutcome::ClearAll {
                    keep_session_id: keep,
                };
            }
            // First Enter on the "Clear all" row arms the confirmation guard.
            if Some(p.selected) == p.clear_row_index() {
                p.confirm_clear = true;
                return TaskOutcome::Idle;
            }
            let pick = p.selection();
            *picker = None;
            return match pick {
                Some(tp) => TaskOutcome::Pick(tp),
                None => TaskOutcome::Idle,
            };
        }
        KeyCode::Esc => {
            if p.confirm_clear {
                // Cancel just the confirmation, keep the picker open.
                p.confirm_clear = false;
            } else {
                *picker = None;
            }
        }
        KeyCode::Up if !p.confirm_clear => {
            p.move_up();
        }
        KeyCode::Down if !p.confirm_clear => {
            p.move_down();
        }
        _ => {}
    }
    TaskOutcome::Idle
}

/// Render the task picker as a centered popup.
pub fn render_task_picker(f: &mut Frame, area: Rect, picker: &TaskPicker) {
    let visible = picker.row_count();
    let want_h = (visible as u16 + 4)
        .min(area.height.saturating_sub(2))
        .max(7);
    let h = want_h.min(area.height.saturating_sub(2));
    let w = 60u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let mut items: Vec<ListItem> = Vec::with_capacity(visible);

    // "+ New task" row
    let new_style = if picker.selected == 0 {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };
    items.push(ListItem::new(Line::from(vec![
        Span::styled("+ ", new_style),
        Span::styled("New task", new_style),
    ])));

    // Session rows
    for (i, s) in picker.sessions.iter().enumerate() {
        let selected = picker.selected == i + 1;
        let is_current = s.id == picker.current_session_id;
        let agent = s.agent.as_deref().unwrap_or("act");
        let title = s.title.as_deref().unwrap_or("(untitled)");
        let style = if selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let mut spans = vec![
            Span::styled(
                format!("[{agent}] "),
                Style::default().fg(crate::render::agent_chip_fg(agent)),
            ),
            Span::styled(title.to_string(), style),
            Span::styled(
                format!("  {}", short_preview(&s.preview)),
                Style::default().fg(Color::DarkGray),
            ),
        ];
        if is_current {
            spans.push(Span::styled(
                "  (current)".to_string(),
                Style::default().fg(Color::Cyan),
            ));
        }
        items.push(ListItem::new(Line::from(spans)));
    }

    // "Clear all" danger row (only when there is something deletable).
    if let Some(clear_idx) = picker.clear_row_index() {
        let deletable = picker.deletable_count();
        let clear_style = if picker.selected == clear_idx {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Red)
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled("\u{2715} ", clear_style),
            Span::styled(format!("Clear all {deletable} task(s)",), clear_style),
        ])));
    }

    let title = if picker.confirm_clear {
        // Red confirmation banner while waiting for the second Enter.
        Line::from(Span::styled(
            format!(
                " \u{26a0} Clear ALL {} task(s)? Enter=confirm, Esc=cancel ",
                picker.deletable_count()
            ),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from(" Tasks (\u{2191}/\u{2193} select, Enter=switch, Esc=cancel) ")
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{276f} ");

    let mut state = ListState::default();
    if visible > 0 {
        state.select(Some(picker.selected));
    }
    f.render_stateful_widget(list, popup, &mut state);
}

fn short_preview(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= 40 {
        t.to_string()
    } else {
        format!("{}...", t.chars().take(40).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn item(id: &str) -> SessionListItem {
        SessionListItem {
            id: id.to_string(),
            title: Some(format!("title-{id}")),
            agent: Some("act".into()),
            model: None,
            created_at: 0,
            updated_at: 0,
            preview: String::new(),
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn picker_with(sessions: Vec<&str>, current: &str) -> Option<TaskPicker> {
        Some(TaskPicker::new(
            sessions.iter().map(|s| item(s)).collect(),
            current.to_string(),
        ))
    }

    #[test]
    fn clear_row_hidden_when_nothing_deletable() {
        // Only the current session exists: nothing to clear.
        let p = TaskPicker::new(vec![item("cur")], "cur".into());
        assert_eq!(p.row_count(), 2, "New + 1 session, no clear row");
        assert!(p.clear_row_index().is_none());
        assert_eq!(p.deletable_count(), 0);
    }

    #[test]
    fn clear_row_shown_when_other_sessions_exist() {
        let p = TaskPicker::new(vec![item("cur"), item("old")], "cur".into());
        assert_eq!(p.row_count(), 4, "New + 2 sessions + clear row");
        assert_eq!(p.clear_row_index(), Some(3));
        assert_eq!(p.deletable_count(), 1);
    }

    #[test]
    fn first_enter_on_clear_row_arms_confirmation() {
        let mut picker = picker_with(vec!["cur", "old"], "cur");
        // Move selection down to the clear row (index 3): 0 New,1 cur,2 old,3 clear.
        for _ in 0..3 {
            handle_task_key(&mut picker, key(KeyCode::Down));
        }
        assert_eq!(picker.as_ref().unwrap().selected, 3);

        let out = handle_task_key(&mut picker, key(KeyCode::Enter));
        assert!(matches!(out, TaskOutcome::Idle), "first Enter only arms");
        assert!(picker.as_ref().unwrap().confirm_clear);
        // Picker stays open.
        assert!(picker.is_some());
    }

    #[test]
    fn second_enter_emits_clear_all_with_keep() {
        let mut picker = picker_with(vec!["cur", "old"], "cur");
        // Arm the confirmation.
        for _ in 0..3 {
            handle_task_key(&mut picker, key(KeyCode::Down));
        }
        handle_task_key(&mut picker, key(KeyCode::Enter));
        // Second Enter commits.
        let out = handle_task_key(&mut picker, key(KeyCode::Enter));
        match out {
            TaskOutcome::ClearAll { keep_session_id } => {
                assert_eq!(keep_session_id, "cur");
            }
            other => panic!("expected ClearAll, got {other:?} unmatched"),
        }
    }

    #[test]
    fn esc_cancels_confirmation_but_keeps_picker_open() {
        let mut picker = picker_with(vec!["cur", "old"], "cur");
        for _ in 0..3 {
            handle_task_key(&mut picker, key(KeyCode::Down));
        }
        handle_task_key(&mut picker, key(KeyCode::Enter)); // arm
        assert!(picker.as_ref().unwrap().confirm_clear);

        handle_task_key(&mut picker, key(KeyCode::Esc));
        assert!(
            !picker.as_ref().unwrap().confirm_clear,
            "Esc cancels confirm"
        );
        assert!(picker.is_some(), "picker still open after Esc");
    }

    #[test]
    fn navigation_locked_during_confirmation() {
        let mut picker = picker_with(vec!["cur", "old"], "cur");
        for _ in 0..3 {
            handle_task_key(&mut picker, key(KeyCode::Down));
        }
        handle_task_key(&mut picker, key(KeyCode::Enter)); // arm confirm
        let before = picker.as_ref().unwrap().selected;
        handle_task_key(&mut picker, key(KeyCode::Up));
        handle_task_key(&mut picker, key(KeyCode::Down));
        assert_eq!(
            picker.as_ref().unwrap().selected,
            before,
            "arrow keys must not move while confirm is armed"
        );
    }

    #[test]
    fn ctrl_c_quits_even_during_confirmation() {
        let mut picker = picker_with(vec!["cur", "old"], "cur");
        for _ in 0..3 {
            handle_task_key(&mut picker, key(KeyCode::Down));
        }
        handle_task_key(&mut picker, key(KeyCode::Enter)); // arm confirm
        let out = handle_task_key(
            &mut picker,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(matches!(out, TaskOutcome::Quit));
    }
}
