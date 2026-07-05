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

pub enum TaskOutcome {
    Idle,
    Quit,
    Pick(TaskPick),
}

/// Modal session picker shown when the user types `/task`.
pub struct TaskPicker {
    sessions: Vec<SessionListItem>,
    selected: usize,
}

impl TaskPicker {
    pub fn new(sessions: Vec<SessionListItem>) -> Self {
        TaskPicker { sessions, selected: 0 }
    }

    fn row_count(&self) -> usize {
        1 + self.sessions.len() // "+ New task" + sessions
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
            self.sessions.get(self.selected - 1).map(|s| TaskPick::Resume(s.id.clone()))
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
            KeyCode::Char('c') | KeyCode::Char('d') => return TaskOutcome::Quit,
            _ => return TaskOutcome::Idle,
        }
    }
    match k.code {
        KeyCode::Up => p.move_up(),
        KeyCode::Down => p.move_down(),
        KeyCode::Enter => {
            let pick = p.selection();
            *picker = None;
            return match pick {
                Some(tp) => TaskOutcome::Pick(tp),
                None => TaskOutcome::Idle,
            };
        }
        KeyCode::Esc => { *picker = None; }
        _ => {}
    }
    TaskOutcome::Idle
}

/// Render the task picker as a centered popup.
pub fn render_task_picker(f: &mut Frame, area: Rect, picker: &TaskPicker) {
    let visible = picker.row_count();
    let want_h = (visible as u16 + 4).min(area.height.saturating_sub(2)).max(7);
    let h = want_h.min(area.height.saturating_sub(2));
    let w = 60u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let mut items: Vec<ListItem> = Vec::with_capacity(visible);

    // "+ New task" row
    let new_style = if picker.selected == 0 {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
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
        let agent = s.agent.as_deref().unwrap_or("act");
        let title = s.title.as_deref().unwrap_or("(untitled)");
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("[{agent}] "), Style::default().fg(Color::Magenta)),
            Span::styled(title.to_string(), style),
            Span::styled(format!("  {}", short_preview(&s.preview)), Style::default().fg(Color::DarkGray)),
        ])));
    }

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Tasks (\u{2191}/\u{2193} select, Enter=switch, Esc=cancel) "),
        )
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
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
