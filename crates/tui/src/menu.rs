//! Skill-selection popup (`$`) for the TUI composer.
//!
//! [`SkillMenu`] holds the picker state: the full skill list, the query the
//! user has typed, the visible rows (filtered), and the highlighted row. When
//! a skill is already active the menu prepends a synthetic "clear" row so the
//! user can un-set the skill. All state transitions go through small methods
//! so the modal handling in `app.rs` stays a flat `match`. [`render_skill_popup`]
//! draws the centered overlay, reusing the `Clear` + centered-`Rect` pattern
//! from the help popup.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::Skill;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

/// Outcome of a keystroke while the skill menu is open. The caller maps this to
/// its own `KeyAction`: `Quit` propagates, `Pick(None)` clears the active skill,
/// `Pick(Some)` activates a skill, and `Idle` leaves the menu open.
pub enum MenuOutcome {
    Idle,
    Quit,
    Pick(Option<(String, String)>),
}

/// Handle one keystroke against an open skill menu, mutating `menu` in place.
/// When the menu is closed by the user (`Esc`, `Enter` on an empty list, or a
/// pick) the `Option` is set to `None` so the caller drops out of modal mode.
pub fn handle_menu_key(menu: &mut Option<SkillMenu>, k: KeyEvent) -> MenuOutcome {
    let m = match menu.as_mut() {
        Some(m) => m,
        None => return MenuOutcome::Idle,
    };
    // Quit (Ctrl+D) still works from inside the modal; other Ctrl combos are ignored.
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(k.code, KeyCode::Char('d') | KeyCode::Char('\u{4}')) {
            *menu = None;
            return MenuOutcome::Quit;
        }
        return MenuOutcome::Idle;
    }
    match k.code {
        KeyCode::Up => m.move_up(),
        KeyCode::Down => m.move_down(),
        KeyCode::Backspace => m.on_backspace(),
        KeyCode::Char(c) => m.on_char(c),
        KeyCode::Enter | KeyCode::Tab => {
            if m.is_clear_selected() {
                *menu = None;
                return MenuOutcome::Pick(None);
            }
            if let Some(s) = m.selected_skill() {
                let name = s.name.clone();
                let body = s.body.clone();
                *menu = None;
                return MenuOutcome::Pick(Some((name, body)));
            }
            // Nothing selectable (empty list) — just close.
            *menu = None;
        }
        KeyCode::Esc => {
            *menu = None;
        }
        _ => {}
    }
    MenuOutcome::Idle
}

/// One display row of the picker. `Clear` un-sets the active skill; `Skill(i)`
/// references `SkillMenu::skills[i]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Clear,
    Skill(usize),
}

/// Picker state for the `$` skill menu.
pub struct SkillMenu {
    skills: Vec<Skill>,
    /// Visible rows (after filtering). Always begins with `Clear` when
    /// `has_active` is true.
    rows: Vec<Row>,
    selected: usize,
    query: String,
    has_active: bool,
}

impl SkillMenu {
    /// Open a new picker. `has_active` controls whether a leading "clear"
    /// row is shown (i.e. whether there's an active skill to un-set).
    pub fn new(skills: Vec<Skill>, has_active: bool) -> Self {
        let rows = Self::build_rows(&skills, "", has_active);
        SkillMenu {
            skills,
            rows,
            selected: 0,
            query: String::new(),
            has_active,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// `true` if the highlighted row is the synthetic "clear" row.
    pub fn is_clear_selected(&self) -> bool {
        self.rows.get(self.selected) == Some(&Row::Clear)
    }

    /// The skill under the highlight, if the highlight isn't on "clear".
    pub fn selected_skill(&self) -> Option<&Skill> {
        match self.rows.get(self.selected) {
            Some(Row::Skill(i)) => self.skills.get(*i),
            _ => None,
        }
    }

    fn visible_count(&self) -> usize {
        self.rows.len()
    }

    /// Move the highlight up, wrapping to the bottom.
    pub fn move_up(&mut self) {
        let n = self.visible_count();
        if n > 0 {
            self.selected = (self.selected + n - 1) % n;
        }
    }

    /// Move the highlight down, wrapping to the top.
    pub fn move_down(&mut self) {
        let n = self.visible_count();
        if n > 0 {
            self.selected = (self.selected + 1) % n;
        }
    }

    /// Append a typed character to the query and re-filter.
    pub fn on_char(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    /// Remove the last query character and re-filter.
    pub fn on_backspace(&mut self) {
        self.query.pop();
        self.refilter();
    }

    /// Rebuild `rows` from the current query (case-insensitive substring match
    /// on name, then description) and clamp the selection. The "clear" row is
    /// always shown when `has_active`, regardless of the query.
    fn refilter(&mut self) {
        self.rows = Self::build_rows(&self.skills, &self.query, self.has_active);
        if self.visible_count() == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.visible_count() - 1);
        }
    }

    fn build_rows(skills: &[Skill], query: &str, has_active: bool) -> Vec<Row> {
        let q = query.trim().to_lowercase();
        let mut rows = Vec::new();
        if has_active {
            rows.push(Row::Clear);
        }
        if q.is_empty() {
            for (i, _s) in skills.iter().enumerate() {
                rows.push(Row::Skill(i));
            }
            return rows;
        }
        // Fuzzy subsequence match on name (primary) or description (fallback).
        // Sort by match score (compact matches rank higher).
        let mut scored: Vec<(usize, i32)> = skills
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let name_l = s.name.to_lowercase();
                let desc_l = s.description.to_lowercase();
                let score = fuzzy_score(&q, &name_l).or_else(|| fuzzy_score(&q, &desc_l))?;
                Some((i, score))
            })
            .collect();
        scored.sort_by_key(|&(_, sc)| sc);
        for (i, _) in scored {
            rows.push(Row::Skill(i));
        }
        rows
    }
}

/// Fuzzy subsequence match: returns `Some(score)` if `query` is a subsequence
/// of `target` (both lowercased by caller). Score is lower = better: rewards
/// compact, consecutive, and early matches. `None` = no match.
pub fn fuzzy_score(query: &str, target: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q_chars: Vec<char> = query.chars().collect();
    let t_chars: Vec<char> = target.chars().collect();
    if q_chars.len() > t_chars.len() {
        return None;
    }
    let mut qi = 0usize;
    let mut score: i32 = 0;
    let mut prev_match: Option<usize> = None;
    for (ti, &tc) in t_chars.iter().enumerate() {
        if qi >= q_chars.len() {
            break;
        }
        if tc == q_chars[qi] {
            // Consecutive match bonus
            if let Some(p) = prev_match {
                if ti == p + 1 {
                    score -= 10; // bonus for consecutive
                }
            }
            // Early match bonus (first few chars of target)
            if ti < 3 {
                score -= 5;
            }
            score += ti as i32; // earlier = lower score = better
            prev_match = Some(ti);
            qi += 1;
        }
    }
    if qi == q_chars.len() {
        Some(score)
    } else {
        None
    }
}

/// Draw the skill picker as a centered overlay on top of the current frame.
pub fn render_skill_popup(f: &mut Frame, area: Rect, menu: &SkillMenu) {
    let popup = centered_popup(area, menu.visible_count());
    f.render_widget(Clear, popup);

    let block = Block::default().borders(Borders::ALL).title(
        " Select skill (\u{2191}/\u{2193} move, type to filter, Enter/Tab=confirm, Esc=cancel) ",
    );

    let skill_rows: Vec<ListItem> = menu
        .rows
        .iter()
        .map(|row| match row {
            Row::Clear => ListItem::new(Line::from(Span::styled(
                "\u{2717} clear skill",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ))),
            Row::Skill(i) => {
                let s = &menu.skills[*i];
                ListItem::new(Line::from(vec![
                    Span::styled(
                        s.name.clone(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" \u{2014} "),
                    Span::styled(s.description.clone(), Style::default().fg(Color::Gray)),
                ]))
            }
        })
        .collect();

    let no_skills_row = if menu.skills.is_empty() {
        Some(ListItem::new(Line::from(Span::styled(
            "  no skills \u{2014} add *.md or <name>/SKILL.md under ~/.opencoder/skills",
            Style::default().fg(Color::DarkGray),
        ))))
    } else {
        None
    };

    let items = if skill_rows.is_empty() {
        no_skills_row.into_iter().collect()
    } else {
        skill_rows
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

    // Query footer so the user can see what they've typed while filtering.
    let footer = Rect::new(
        popup.x,
        popup.bottom(),
        popup.width,
        1u16.min(area.height.saturating_sub(popup.bottom())),
    );
    if footer.height > 0 {
        let footer_line = Line::from(vec![
            Span::styled(" filter: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                menu.query(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("_"),
        ]);
        f.render_widget(
            Paragraph::new(footer_line).wrap(Wrap { trim: false }),
            footer,
        );
    }
}

/// Render the skill menu into a pre-computed rect (dropdown style — anchored
/// above the composer, not centered). Used by the new layout.
pub fn render_skill_in_rect(f: &mut Frame, rect: Rect, menu: &SkillMenu) {
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Skills (\u{2191}/\u{2193} select, type to filter, Enter/Tab=confirm) ");

    let skill_rows: Vec<ListItem> = menu
        .rows
        .iter()
        .map(|row| match row {
            Row::Clear => ListItem::new(Line::from(Span::styled(
                "\u{2717} clear skill",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ))),
            Row::Skill(i) => {
                let s = &menu.skills[*i];
                ListItem::new(Line::from(vec![
                    Span::styled(
                        s.name.clone(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" \u{2014} "),
                    Span::styled(s.description.clone(), Style::default().fg(Color::Gray)),
                ]))
            }
        })
        .collect();

    let items = if skill_rows.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  no skills \u{2014} add *.md under ~/.opencoder/skills",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        skill_rows
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
    f.render_stateful_widget(list, rect, &mut state);
}

/// Compute a centered popup rect sized to the menu content.
fn centered_popup(area: Rect, visible: usize) -> Rect {
    let max_h = area.height.saturating_sub(2);
    // +5: 2 borders + footer query line + breathing room.
    let want_h = (visible as u16).saturating_add(5).min(max_h).max(7);
    let h = want_h.min(max_h);
    let w = 70u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sk(name: &str, desc: &str) -> Skill {
        Skill {
            name: name.into(),
            description: desc.into(),
            body: format!("body of {name}"),
            source: PathBuf::from(format!("/tmp/{name}.md")),
        }
    }

    fn menu_of(names: &[&str]) -> SkillMenu {
        SkillMenu::new(
            names.iter().map(|n| sk(n, &format!("desc {n}"))).collect(),
            false,
        )
    }

    #[test]
    fn opens_with_all_visible_and_first_selected() {
        let m = menu_of(&["alpha", "beta"]);
        assert_eq!(m.visible_count(), 2);
        assert_eq!(m.selected_skill().unwrap().name, "alpha");
        assert!(!m.is_clear_selected());
    }

    #[test]
    fn prepend_clear_row_when_active() {
        let mut m = SkillMenu::new(vec![sk("alpha", "a")], true);
        assert_eq!(m.visible_count(), 2);
        assert!(m.is_clear_selected()); // first row is clear
        m.move_down();
        assert_eq!(m.selected_skill().unwrap().name, "alpha");
    }

    #[test]
    fn move_down_wraps_and_move_up_wraps() {
        let mut m = menu_of(&["a", "b", "c"]);
        m.move_down();
        assert_eq!(m.selected_skill().unwrap().name, "b");
        m.move_down();
        assert_eq!(m.selected_skill().unwrap().name, "c");
        m.move_down();
        assert_eq!(m.selected_skill().unwrap().name, "a");
        m.move_up();
        assert_eq!(m.selected_skill().unwrap().name, "c");
    }

    #[test]
    fn query_filters_by_name_or_description_case_insensitive() {
        let mut m = SkillMenu::new(
            vec![
                sk("repo-memory", "maintain local docs"),
                sk("task-plan", "build a go-live plan"),
                sk("ship", "deliver to production"),
            ],
            false,
        );
        for c in "PLAN".chars() {
            m.on_char(c);
        }
        assert_eq!(m.visible_count(), 1);
        assert_eq!(m.selected_skill().unwrap().name, "task-plan");
        assert_eq!(m.query(), "PLAN");
    }

    #[test]
    fn backspace_removes_last_filter_char() {
        let mut m = menu_of(&["alpha", "beta", "gamma"]);
        m.on_char('a');
        assert_eq!(m.visible_count(), 3); // alpha, beta, gamma all contain 'a'
        m.on_backspace();
        assert_eq!(m.visible_count(), 3); // filter cleared, all visible
    }

    #[test]
    fn selection_clamps_after_filter_shrinks() {
        let mut m = menu_of(&["a", "b", "c"]);
        m.move_down();
        m.move_down(); // "c"
        assert_eq!(m.selected_skill().unwrap().name, "c");
        m.on_char('a'); // only "a" matches
        assert_eq!(m.selected_skill().unwrap().name, "a");
    }

    #[test]
    fn empty_list_is_empty_and_selects_nothing() {
        let m = SkillMenu::new(vec![], false);
        assert!(m.is_empty());
        assert!(m.selected_skill().is_none());
    }

    #[test]
    fn empty_menu_moves_without_panicking() {
        let mut m = SkillMenu::new(vec![], false);
        m.move_up();
        m.move_down();
        assert!(m.selected_skill().is_none());
    }

    #[test]
    fn clear_row_stays_visible_through_filter_when_active() {
        let mut m = SkillMenu::new(vec![sk("alpha", "a"), sk("beta", "b")], true);
        for c in "zzz".chars() {
            m.on_char(c);
        } // matches nothing
          // clear row always remains, skills filtered out
        assert_eq!(m.visible_count(), 1);
        assert!(m.is_clear_selected());
    }

    #[test]
    fn fuzzy_score_subsequence_match() {
        assert!(fuzzy_score("exp", "explore").is_some());
        assert!(fuzzy_score("exp", "build").is_none());
        assert!(fuzzy_score("", "anything").is_some());
        assert!(fuzzy_score("abc", "axbxc").is_some());
        assert!(
            fuzzy_score("abc", "acb").is_none(),
            "order must be preserved"
        );
    }

    #[test]
    fn fuzzy_score_ranks_compact_matches_higher() {
        let s1 = fuzzy_score("exp", "explore").unwrap();
        let s2 = fuzzy_score("exp", "e_x_p").unwrap();
        assert!(s1 < s2, "compact match should score better (lower)");
    }

    #[test]
    fn fuzzy_filter_matches_subsequence_not_substring() {
        let mut m = menu_of(&["explore", "build"]);
        for c in "epr".chars() {
            m.on_char(c);
        }
        // "epr" is a subsequence of "explore" (e-x-p-l-o-r-e) but NOT of "build"
        let names: Vec<&str> = m
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Skill(i) => Some(m.skills[*i].name.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            names.contains(&"explore"),
            "explore should match 'epr': {:?}",
            names
        );
        assert!(!names.contains(&"build"), "build should NOT match 'epr'");
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    #[test]
    fn enter_picks_highlighted_skill_and_closes_menu() {
        let mut menu = Some(menu_of(&["alpha", "beta"]));
        let outcome = handle_menu_key(&mut menu, key(KeyCode::Enter));
        match outcome {
            MenuOutcome::Pick(Some((name, body))) => {
                assert_eq!(name, "alpha");
                assert_eq!(body, "body of alpha");
            }
            _ => panic!("Enter should confirm the highlighted skill"),
        }
        assert!(menu.is_none(), "menu must close after a pick");
    }

    #[test]
    fn tab_picks_highlighted_skill_like_enter() {
        let mut menu = Some(menu_of(&["alpha", "beta"]));
        // move down first so a non-default item is highlighted; Tab must pick it.
        let m = menu.as_mut().unwrap();
        m.move_down();
        let outcome = handle_menu_key(&mut menu, key(KeyCode::Tab));
        match outcome {
            MenuOutcome::Pick(Some((name, _))) => assert_eq!(name, "beta"),
            _ => panic!("Tab should confirm the highlighted skill"),
        }
        assert!(menu.is_none(), "menu must close on Tab pick");
    }

    #[test]
    fn tab_picks_clear_row_like_enter() {
        let mut menu = Some(SkillMenu::new(vec![sk("alpha", "a")], true));
        // clear row is first and selected by default.
        let outcome = handle_menu_key(&mut menu, key(KeyCode::Tab));
        assert!(matches!(outcome, MenuOutcome::Pick(None)));
        assert!(menu.is_none());
    }

    #[test]
    fn tab_on_empty_menu_just_closes() {
        let mut menu = Some(SkillMenu::new(vec![], false));
        let outcome = handle_menu_key(&mut menu, key(KeyCode::Tab));
        assert!(matches!(outcome, MenuOutcome::Idle));
        assert!(menu.is_none());
    }
}
