//! Agent picker for the `/agent` slash command — a dropdown anchored just
//! above the composer (same geometry as `file_menu::render_file_popup`).
//!
//! Lists the switchable primary agents: builtin roles (act/plan/command —
//! the `workflow` scheduler is Primary but excluded everywhere consumers
//! offer switch targets) plus the file-based cards from
//! `opencoder_core::list_agents`, each carrying its one-line identity from
//! [`opencoder_core::agent_description`] (prompt-pool soul.md first line).
//! Rows filter through the same fuzzy matcher the `$` skill picker uses
//! (`menu::fuzzy_score`), name first with the description as fallback —
//! 1:1 with the SPA `@` agent menu. A pick fills the composer with
//! `/agent <name> ` (trailing space): the text then rides the normal submit
//! path and the runner's control head applies the switch
//! (`opencoder_session::control_cmd::split_control_prefix`), so the picker
//! itself owns no I/O.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::AgentMode;
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

/// Switchable primary agents for the picker: builtin primary roles first
/// (act/plan/command), then the resolvable file-agent cards. Mirrors the
/// web `/api/agents` `primary` computation
/// (`is_primary() && name != "workflow"`) so both surfaces offer the same
/// switch targets — the switch endpoint rejects anything else.
pub fn available_primary_agents() -> Vec<AgentCard> {
    let mut cards: Vec<AgentCard> = opencoder_core::builtin_agents()
        .into_iter()
        .filter(|a| a.mode == AgentMode::Primary && a.name != "workflow")
        .map(|a| AgentCard {
            name: a.name,
            description: a.description,
        })
        .collect();
    for name in opencoder_core::agent::list_agents() {
        if cards.iter().any(|c| c.name == name) {
            continue; // builtin names can never be shadowed by file cards
        }
        let description = opencoder_core::agent::agent_description(&name)
            .unwrap_or_else(|| format!("Custom agent {name}"));
        cards.push(AgentCard { name, description });
    }
    cards
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
        // Enter and Tab both confirm — same as the `$` skill picker.
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
            "  no matching agent",
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
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn card(name: &str, desc: &str) -> AgentCard {
        AgentCard {
            name: name.into(),
            description: desc.into(),
        }
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn menu_with(cards: &[(&str, &str)]) -> AgentMenu {
        AgentMenu::new(cards.iter().map(|(n, d)| card(n, d)).collect())
    }

    #[test]
    fn empty_query_lists_every_agent_in_order() {
        let m = AgentMenu::new(vec![card("act", "d"), card("writer", "w"), card("plan", "p")]);
        assert_eq!(m.visible_count(), 3);
        assert_eq!(
            m.visible_agents().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["act", "writer", "plan"]
        );
    }

    #[test]
    fn fuzzy_filter_matches_the_name_first_and_description_as_fallback() {
        let mut m = AgentMenu::new(vec![
            card("coder", "Custom agent coder"),
            card("writer", "small diffs"),
            card("plan", "read-only explorer"),
        ]);
        for c in "wr".chars() {
            m.on_char(c);
        }
        // 'wr' is a subsequence of 'writer' but of no other name/description
        // here — the row set narrows to the name match.
        assert_eq!(m.visible_count(), 1);
        assert_eq!(m.selected_agent().unwrap().name, "writer");
        // Description fallback: a query that misses every name can still hit
        // a description ('read-only explorer' fuzzy-matches 'explr').
        let mut m = AgentMenu::new(vec![
            card("coder", "bash and subagents"),
            card("plan", "Read-only plan agent. Explores code."),
        ]);
        for c in "explr".chars() {
            m.on_char(c);
        }
        assert_eq!(m.visible_count(), 1);
        assert_eq!(m.selected_agent().unwrap().name, "plan");
    }

    #[test]
    fn best_fuzzy_score_sorts_first_and_a_miss_empties_the_menu() {
        let mut m = AgentMenu::new(vec![
            card("c_o_d_r", "scattered"),
            card("coder", "compact prefix"),
            card("sandy", "no match"),
        ]);
        for c in "cod".chars() {
            m.on_char(c);
        }
        assert_eq!(
            m.visible_agents().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["coder", "c_o_d_r"],
            "compact prefix must rank ahead of the scattered subsequence"
        );
        for _ in 0..3 {
            m.on_backspace();
        }
        for c in "zzz".chars() {
            m.on_char(c);
        }
        assert_eq!(m.visible_count(), 0, "no subsequence, no rows");
        assert_eq!(m.selected_agent(), None);
    }

    #[test]
    fn enter_and_tab_pick_the_highlighted_agent_and_close_the_menu() {
        for close_key in [KeyCode::Enter, KeyCode::Tab] {
            let mut slot = Some(AgentMenu::new(vec![card("writer", "w"), card("act", "a")]));
            m_down(&mut slot);
            let outcome = handle_agent_key(&mut slot, KeyEvent::new(close_key, KeyModifiers::NONE));
            assert_eq!(outcome, AgentOutcome::Pick("act".into()));
            assert!(slot.is_none(), "pick closes the menu");
        }
    }

    #[test]
    fn esc_closes_without_picking_and_ctrl_d_quits() {
        let mut slot = Some(AgentMenu::new(vec![card("act", "a")]));
        assert_eq!(
            handle_agent_key(&mut slot, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            AgentOutcome::Idle
        );
        assert!(slot.is_none());
        let mut slot = Some(AgentMenu::new(vec![card("act", "a")]));
        assert_eq!(
            handle_agent_key(&mut slot, KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            AgentOutcome::Quit
        );
        assert!(slot.is_none());
        // Closed slot: every keystroke is Idle (caller's modal mode owns it).
        let mut none = None;
        assert_eq!(handle_agent_key(&mut none, key('a')), AgentOutcome::Idle);
    }

    fn m_down(slot: &mut Option<AgentMenu>) {
        if let Some(m) = slot.as_mut() {
            m.move_down();
        }
    }

    #[test]
    fn pick_token_carries_the_control_head_with_trailing_space() {
        assert_eq!(pick_token("writer"), "/agent writer ");
    }

    #[test]
    fn available_cards_start_with_builtin_primaries_without_workflow() {
        let cards = available_primary_agents();
        let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            &names[..3.min(names.len())],
            &["act", "plan", "command"][..3.min(names.len())]
        );
        assert!(!names.contains(&"workflow"), "scheduler role is excluded");
        assert!(!names.contains(&"explore"), "subagents are not switchable");
        // Without an agents-root override there are no file cards, so builtin
        // descriptions come through verbatim.
        let act = cards.iter().find(|c| c.name == "act").unwrap();
        assert!(act.description.contains("Default execution agent"));
    }

    #[test]
    fn file_agents_merge_in_with_soul_description_and_generic_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let (_lock, _guard) = override_agents(dir.path());
        // writer: a real prompt pool with a soul.md first line.
        write_file_agent(dir.path(), "writer", "Writer soul: small diffs.\nmore");
        // bare: a card with no prompt reference → generic fallback.
        std::fs::create_dir_all(dir.path().join("bare")).unwrap();
        std::fs::write(
            dir.path().join("bare").join("meta.json"),
            r#"{ "name": "bare", "current": { "prompt": "missing" } }"#,
        )
        .unwrap();

        let cards = available_primary_agents();
        let writer = cards.iter().find(|c| c.name == "writer").expect("writer listed");
        assert_eq!(writer.description, "Writer soul: small diffs.");
        let bare = cards.iter().find(|c| c.name == "bare").expect("bare listed");
        assert_eq!(bare.description, "Custom agent bare");
    }

    // -- fixtures ----------------------------------------------------------

    /// Override guard that RESETS the process-global root on drop, so the
    /// next test in this binary never sees a deleted tempdir.
    struct AgentRootGuard;

    impl Drop for AgentRootGuard {
        fn drop(&mut self) {
            opencoder_core::agent::set_agents_dir_override(None);
        }
    }

    fn override_agents(root: &std::path::Path) -> (std::sync::MutexGuard<'static, ()>, AgentRootGuard) {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        opencoder_core::agent::set_agents_dir_override(Some(root.to_path_buf()));
        (g, AgentRootGuard)
    }

    fn write_file_agent(root: &std::path::Path, name: &str, soul: &str) {
        let pool = root.join("prompts").join(name);
        let vdir = pool.join("v1");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("soul.md"), soul).unwrap();
        std::fs::write(
            pool.join("meta.json"),
            format!(r#"{{ "name": "{name}", "current": 1, "history": [1] }}"#),
        )
        .unwrap();
        let adir = root.join(name);
        std::fs::create_dir_all(&adir).unwrap();
        std::fs::write(
            adir.join("meta.json"),
            format!(r#"{{ "name": "{name}", "current": {{ "prompt": "{name}" }} }}"#),
        )
        .unwrap();
    }
}
