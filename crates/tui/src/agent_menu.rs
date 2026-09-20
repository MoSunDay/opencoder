//! Agent picker for the `/agent` slash command -- a dropdown anchored just
//! above the composer (same geometry as `file_menu::render_file_popup`).
//!
//! Lists only custom file-based cards from `opencoder_core::list_agents`,
//! excluding every builtin name even if a same-named directory exists.
//! Runtime mode switching has its own `/act` and `/plan` commands.
//! Each card carries its one-line identity from
//! [`opencoder_core::agent_description`] (prompt-pool soul.md first line).
//! Rows filter through the same fuzzy matcher the `$` skill picker uses
//! (`menu::fuzzy_score`), name first with the description as fallback --
//! 1:1 with the SPA `@` agent menu. A pick fills the composer with
//! `/agent <name> ` (trailing space): the text then rides the normal submit
//! path and the runner's control head applies the switch
//! (`opencoder_session::control_cmd::split_control_prefix`), so the picker
//! itself owns no I/O.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::menu::fuzzy_score;
use crate::theme;

/// One display row: agent name + one-line identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCard {
    pub name: String,
    pub description: String,
}

/// Custom agent cards in directory-name order. Builtin roles stay out of
/// this picker, including file cards whose names would resolve to builtins.
pub fn available_primary_agents() -> Vec<AgentCard> {
    let builtins = opencoder_core::builtin_agents();
    opencoder_core::agent::list_agents()
        .into_iter()
        .filter(|name| !builtins.iter().any(|agent| agent.name == *name))
        .map(|name| {
            let description = opencoder_core::agent::agent_description(&name)
                .unwrap_or_else(|| format!("Custom agent {name}"));
            AgentCard { name, description }
        })
        .collect()
}

/// The composer text a pick produces: the runner's `/agent <name>` control
/// head with a trailing space (args may follow as a compound prompt).
pub fn pick_token(name: &str) -> String {
    format!("/agent {name} ")
}

/// Outcome of a keystroke while the agent menu is open. `Quit` propagates
/// Ctrl+D; `Pick` carries the chosen agent name.
#[derive(Debug, PartialEq, Eq)]
pub enum AgentOutcome {
    Idle,
    Quit,
    Pick(String),
}

/// Picker state for the `/agent` menu (mirrors `SkillMenu` in `menu.rs`).
#[derive(Debug)]
pub struct AgentMenu {
    agents: Vec<AgentCard>,
    /// Visible row indices into `agents`, best fuzzy score first.
    rows: Vec<usize>,
    selected: usize,
    query: String,
}

impl AgentMenu {
    pub fn new(agents: Vec<AgentCard>) -> Self {
        let mut m = Self {
            agents,
            rows: Vec::new(),
            selected: 0,
            query: String::new(),
        };
        m.refilter();
        m
    }

    pub fn visible_count(&self) -> usize {
        self.rows.len()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn visible_agents(&self) -> impl Iterator<Item = &AgentCard> {
        self.rows.iter().filter_map(|i| self.agents.get(*i))
    }

    /// The agent under the highlight, if any.
    pub fn selected_agent(&self) -> Option<&AgentCard> {
        self.agents.get(*self.rows.get(self.selected)?)
    }

    pub fn selected_row(&self) -> usize {
        self.selected
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

    fn refilter(&mut self) {
        let q = self.query.trim().to_lowercase();
        if q.is_empty() {
            self.rows = (0..self.agents.len()).collect();
        } else {
            let mut scored: Vec<(usize, i32)> = self
                .agents
                .iter()
                .enumerate()
                .filter_map(|(i, card)| {
                    let score = fuzzy_score(&q, &card.name.to_lowercase())
                        .or_else(|| fuzzy_score(&q, &card.description.to_lowercase()))?;
                    Some((i, score))
                })
                .collect();
            scored.sort_by_key(|(_, s)| *s);
            self.rows = scored.into_iter().map(|(i, _)| i).collect();
        }
        self.selected = if self.rows.is_empty() {
            0
        } else {
            self.selected.min(self.rows.len() - 1)
        };
    }
}

/// Handle one keystroke against an open agent menu, mutating the slot in
/// place (same contract as `menu::handle_menu_key`).
pub fn handle_agent_key(menu: &mut Option<AgentMenu>, k: KeyEvent) -> AgentOutcome {
    let m = match menu.as_mut() {
        Some(m) => m,
        None => return AgentOutcome::Idle,
    };
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(k.code, KeyCode::Char('d') | KeyCode::Char('\u{4}')) {
            *menu = None;
            return AgentOutcome::Quit;
        }
        return AgentOutcome::Idle;
    }
    match k.code {
        KeyCode::Up => m.move_up(),
        KeyCode::Down => m.move_down(),
        KeyCode::Char(c) => m.on_char(c),
        KeyCode::Backspace => m.on_backspace(),
        // Enter and Tab both confirm -- same as the `$` skill picker.
        KeyCode::Enter | KeyCode::Tab => {
            let name = m.selected_agent().map(|c| c.name.clone());
            *menu = None;
            return match name {
                Some(n) => AgentOutcome::Pick(n),
                None => AgentOutcome::Idle,
            };
        }
        KeyCode::Esc => {
            *menu = None;
            return AgentOutcome::Idle;
        }
        _ => {}
    }
    AgentOutcome::Idle
}

/// Draw the picker: box + rows + `/query` footer, bottom edge above the
/// composer (file-mention picker geometry).
pub fn render_agent_popup(f: &mut Frame, area: Rect, composer_top: u16, menu: &AgentMenu) {
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

    let block = theme::rounded_block(
        "/agent (\u{2191}/\u{2193} move, type to filter, Enter/Tab=switch, Esc=cancel)",
    );

    let items: Vec<ListItem> = menu
        .visible_agents()
        .map(|c| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {} ", c.name),
                    Style::default()
                        .fg(theme::accent())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(c.description.clone(), Style::default().fg(theme::muted())),
            ]))
        })
        .collect();

    let items = if items.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            if menu.agents.is_empty() {
                "  no custom agents available"
            } else {
                "  no matching agent"
            },
            Style::default().fg(theme::muted()),
        )))]
    } else {
        items
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(theme::list_highlight())
        .highlight_symbol("\u{276f} ");

    let mut state = ListState::default();
    if menu.visible_count() > 0 {
        state.select(Some(menu.selected_row()));
    }
    f.render_stateful_widget(list, popup, &mut state);

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
                menu.query.clone(),
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
#[path = "agent_menu_tests.rs"]
mod tests;
