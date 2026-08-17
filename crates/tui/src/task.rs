//! `/task` session picker — switch between or create new conversations.

use crate::composer;
use crate::theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_store::SessionListItem;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState};
use ratatui::Frame;

/// What the user picked from the task picker.
#[derive(Clone, Debug)]
pub enum TaskPick {
    New,
    Resume(String),
    /// Fork (clone) the selected session's context into a brand-new session.
    Fork(String),
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

/// Which interaction mode the picker is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerMode {
    /// `/task` — switch to / create a session ("+ New task", "Clear all" rows).
    Switch,
    /// `/fork` — pick a session to clone context from (sessions only, no
    /// "+ New task" / "Clear all" rows).
    Fork,
}

/// Modal session picker shown when the user types `/task` or `/fork`.
pub struct TaskPicker {
    sessions: Vec<SessionListItem>,
    selected: usize,
    /// Interaction mode: session switching (default) vs. fork selection.
    mode: PickerMode,
    /// The currently-active session id — always preserved by "Clear all", and
    /// tagged `(current)` in the rendered list.
    current_session_id: String,
    /// Two-step confirmation guard for the destructive "Clear all" row.
    /// `true` while we're waiting for the second Enter (or an Esc to cancel).
    confirm_clear: bool,
    /// Discovered skills, used to resolve a stored skill **body** (what
    /// `sessions.skill` persists) back to a display name for the row tag.
    skills: Vec<opencoder_core::Skill>,
}

impl TaskPicker {
    pub fn new(sessions: Vec<SessionListItem>, current_session_id: String) -> Self {
        Self::with_skills(
            sessions,
            current_session_id,
            opencoder_core::discover_skills(),
        )
    }

    /// Construct with an explicit skill slice so tests can inject a fake
    /// skill without touching `~/.opencoder/skills`.
    fn with_skills(
        sessions: Vec<SessionListItem>,
        current_session_id: String,
        skills: Vec<opencoder_core::Skill>,
    ) -> Self {
        TaskPicker {
            sessions,
            selected: 0,
            mode: PickerMode::Switch,
            current_session_id,
            confirm_clear: false,
            skills,
        }
    }

    /// Build a fork-mode picker (`/fork`): every listed session is a fork
    /// source, and Enter forks the highlighted session's context.
    pub fn new_fork(sessions: Vec<SessionListItem>, current_session_id: String) -> Self {
        let mut p = Self::with_skills(
            sessions,
            current_session_id,
            opencoder_core::discover_skills(),
        );
        p.mode = PickerMode::Fork;
        p
    }

    /// Resolve a stored skill body to a display tag (`[name]`), matching
    /// against the discovered skills (the store persists the body, not the
    /// name). Falls back to the body's first line so an active skill that is
    /// no longer discoverable still renders something.
    fn skill_tag(&self, body: &str) -> Option<String> {
        let name = self
            .skills
            .iter()
            .find(|sk| sk.body == body)
            .map(|sk| sk.name.clone())
            .or_else(|| {
                body.lines()
                    .find(|l| !l.trim().is_empty())
                    .map(|l| l.trim().trim_start_matches('#').trim().to_string())
            })
            .unwrap_or_default();
        let name = composer::truncate_to_width(&name, 18);
        if name.is_empty() {
            None
        } else {
            // `agent_txt` already carries a trailing space, so the tag renders
            // as `[act] [do-and-done]`.
            Some(format!("[{name}]"))
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
        // Fork mode has no "Clear all" row (and no confirm_clear arming).
        if self.mode == PickerMode::Fork {
            return None;
        }
        if self.deletable_count() == 0 {
            return None;
        }
        Some(1 + self.sessions.len())
    }

    fn row_count(&self) -> usize {
        match self.mode {
            // "+ New task" + sessions (+ "Clear all" when anything deletable).
            PickerMode::Switch => {
                let base = 1 + self.sessions.len();
                if self.clear_row_index().is_some() {
                    base + 1
                } else {
                    base
                }
            }
            // Fork mode lists the sessions themselves (no auxiliary rows).
            PickerMode::Fork => self.sessions.len(),
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
        match self.mode {
            PickerMode::Switch => {
                if self.selected == 0 {
                    Some(TaskPick::New)
                } else {
                    self.sessions
                        .get(self.selected - 1)
                        .map(|s| TaskPick::Resume(s.id.clone()))
                }
            }
            PickerMode::Fork => self
                .sessions
                .get(self.selected)
                .map(|s| TaskPick::Fork(s.id.clone())),
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
            KeyCode::Char('d') | KeyCode::Char('\u{4}') => return TaskOutcome::Quit,
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

/// Fixed popup width (in display columns). Session rows budget their
/// agent/title/preview/badge spans against this so status badges stay visible.
const POPUP_W: u16 = 60;

/// Render the task picker as a centered popup.
pub fn render_task_picker(f: &mut Frame, area: Rect, picker: &TaskPicker) {
    let visible = picker.row_count();
    let want_h = (visible as u16 + 4)
        .min(area.height.saturating_sub(2))
        .max(7);
    let h = want_h.min(area.height.saturating_sub(2));
    let w = POPUP_W.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let is_fork = picker.mode == PickerMode::Fork;
    // Session rows start at index 1 behind "+ New task" in switch mode, and
    // at index 0 in fork mode (no auxiliary rows).
    let row_offset = if is_fork { 0 } else { 1 };
    let mut items: Vec<ListItem> = Vec::with_capacity(visible);

    // "+ New task" row (switch mode only)
    if !is_fork {
        let new_style = if picker.selected == 0 {
            Style::default()
                .fg(theme::ok_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::ok_color())
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled("+ ", new_style),
            Span::styled("New task", new_style),
        ])));
    }

    // Session rows
    for (i, s) in picker.sessions.iter().enumerate() {
        let selected = picker.selected == i + row_offset;
        let is_current = s.id == picker.current_session_id;
        let agent = s.agent.as_deref().unwrap_or("act");
        let title = s.title.as_deref().unwrap_or("(untitled)");
        let style = if selected {
            Style::default()
                .fg(theme::warn_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        // Subagent-task status badges, derived from the persisted
        // `subagent_tasks` table: in-flight children (`Running`) and
        // interrupted ones pending replay on the next user turn (`Cancelled`).
        let running_badge =
            (s.subagent_running > 0).then(|| format!("  \u{25cf} {} running", s.subagent_running));
        let cancelled_badge = (s.subagent_cancelled > 0)
            .then(|| format!("  \u{2297} {} replay pending", s.subagent_cancelled));
        let agent_txt = format!("[{agent}] ");
        // The store keeps the active skill body, not its name; resolve a
        // display tag (` [name]`) by matching against the discovered skills so
        // a `[plan]`-mode row can also show e.g. `[do-and-done]`.
        let skill_tag = s
            .skill
            .as_deref()
            .filter(|b| !b.trim().is_empty())
            .and_then(|b| picker.skill_tag(b));

        // Budget the row inside the fixed-width popup so the status badges
        // stay visible: the agent chip, skill tag, separators, badges and
        // suffix tags are fixed overhead; title and preview split whatever
        // width remains.
        let mut fixed = composer::str_width(&agent_txt)
            + 2 // "  " separator before the preview
            + skill_tag.as_deref().map_or(0, composer::str_width)
            + running_badge.as_deref().map_or(0, composer::str_width)
            + cancelled_badge.as_deref().map_or(0, composer::str_width);
        if is_current {
            fixed += composer::str_width("  (current)");
        }
        let free = (POPUP_W as usize).saturating_sub(fixed);
        let title_budget = (free * 2 / 3).clamp(10, 28);
        let preview_budget = free.saturating_sub(title_budget).max(8);

        let mut spans = vec![Span::styled(
            agent_txt,
            Style::default().fg(crate::theme::agent_chip_fg(agent)),
        )];
        if let Some(tag) = skill_tag {
            spans.push(Span::styled(tag, Style::default().fg(theme::accent())));
        }
        spans.push(Span::styled(
            composer::truncate_to_width(title, title_budget),
            style,
        ));
        spans.push(Span::styled(
            format!("  {}", short_preview(&s.preview, preview_budget)),
            Style::default().fg(theme::muted()),
        ));
        if let Some(badge) = running_badge {
            spans.push(Span::styled(
                badge,
                Style::default().fg(theme::warn_color()),
            ));
        }
        if let Some(badge) = cancelled_badge {
            spans.push(Span::styled(badge, Style::default().fg(theme::muted())));
        }
        if is_current {
            spans.push(Span::styled(
                "  (current)".to_string(),
                Style::default().fg(theme::accent()),
            ));
        }
        items.push(ListItem::new(Line::from(spans)));
    }

    // "Clear all" danger row (only when there is something deletable).
    if let Some(clear_idx) = picker.clear_row_index() {
        let deletable = picker.deletable_count();
        let clear_style = if picker.selected == clear_idx {
            Style::default()
                .fg(theme::err_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::err_color())
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
            Style::default()
                .fg(theme::err_color())
                .add_modifier(Modifier::BOLD),
        ))
    } else if is_fork {
        Line::from(" Fork (\u{2191}/\u{2193} select, Enter=fork context, Esc=cancel) ")
    } else {
        Line::from(" Tasks (\u{2191}/\u{2193} select, Enter=switch, Esc=cancel) ")
    };

    let list = List::new(items)
        .block(crate::theme::rounded_block_plain().title(title))
        .highlight_style(crate::theme::list_highlight())
        .highlight_symbol("\u{276f} ");

    let mut state = ListState::default();
    if visible > 0 {
        state.select(Some(picker.selected));
    }
    f.render_stateful_widget(list, popup, &mut state);
}

/// Truncate a session-list preview to `max_w` *display columns*.
fn short_preview(s: &str, max_w: usize) -> String {
    composer::truncate_to_width(s.trim(), max_w)
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
            subagent_running: 0,
            subagent_cancelled: 0,
            skill: None,
        }
    }

    fn busy_item(id: &str, running: usize, cancelled: usize) -> SessionListItem {
        SessionListItem {
            subagent_running: running,
            subagent_cancelled: cancelled,
            ..item(id)
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
    fn ctrl_c_does_not_quit_during_confirmation() {
        let mut picker = picker_with(vec!["cur", "old"], "cur");
        for _ in 0..3 {
            handle_task_key(&mut picker, key(KeyCode::Down));
        }
        handle_task_key(&mut picker, key(KeyCode::Enter)); // arm confirm
        let out = handle_task_key(
            &mut picker,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(
            !matches!(out, TaskOutcome::Quit),
            "Ctrl+C must not quit the task picker"
        );
    }

    // ── Status-badge rendering ────────────────────────────────────────────

    /// Concatenate every cell's symbol row-by-row into one searchable string.
    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        let area = buf.area;
        let mut s = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    fn render_picker_to_text(picker: &TaskPicker) -> String {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render_task_picker(f, f.area(), picker))
            .unwrap();
        buffer_text(terminal.backend().buffer())
    }

    fn render_to_text(sessions: Vec<SessionListItem>) -> String {
        render_picker_to_text(&TaskPicker::new(sessions, "cur".into()))
    }

    fn fake_skill(name: &str, body: &str) -> opencoder_core::Skill {
        opencoder_core::Skill {
            name: name.into(),
            description: String::new(),
            body: body.into(),
            source: std::path::PathBuf::from("fake"),
        }
    }

    #[test]
    fn status_badges_render_running_and_replay_pending() {
        let text = render_to_text(vec![busy_item("s1", 2, 1), busy_item("s2", 0, 0)]);
        assert!(
            text.contains("\u{25cf} 2 running"),
            "running badge missing from picker:\n{text}"
        );
        assert!(
            text.contains("\u{2297} 1 replay pending"),
            "replay-pending badge missing from picker:\n{text}"
        );
        // The idle session must render no status badge at all.
        assert_eq!(
            text.matches("running").count(),
            1,
            "only the busy row should carry a running badge:\n{text}"
        );
        assert_eq!(
            text.matches("replay pending").count(),
            1,
            "only the busy row should carry a replay-pending badge:\n{text}"
        );
    }

    #[test]
    fn status_badges_survive_long_titles_and_suffix_tags() {
        // Worst case: both badges + (current) + a title long enough to need
        // truncation. The badges are placed before the suffix tags, so they
        // must stay on screen even when the row overflows.
        let text = render_to_text(vec![busy_item("s1", 1, 2)]);
        assert!(
            text.contains("\u{25cf} 1 running"),
            "running badge clipped by long title:\n{text}"
        );
        assert!(
            text.contains("\u{2297} 2 replay pending"),
            "replay-pending badge clipped by long title:\n{text}"
        );
    }

    #[test]
    fn skill_tag_renders_matching_name_next_to_mode_chip() {
        // The store persists the skill body; the picker must resolve it back
        // to the skill name via `discover_skills()`-style matching.
        let mut item = item("s1");
        item.skill = Some("## do-and-done\nfull body".into());
        let picker = TaskPicker::with_skills(
            vec![item],
            "cur".into(),
            vec![fake_skill("do-and-done", "## do-and-done\nfull body")],
        );
        let text = render_picker_to_text(&picker);
        assert!(
            text.contains("[do-and-done]"),
            "skill tag must render next to the mode chip:\n{text}"
        );
        assert!(
            text.contains("[act] [do-and-done]"),
            "skill tag must follow the agent chip:\n{text}"
        );
    }

    #[test]
    fn skill_tag_falls_back_to_first_body_line_when_not_discovered() {
        // A skill that is no longer on disk (body unmatched) still renders a
        // derived tag instead of vanishing silently.
        let mut item = item("s1");
        item.skill = Some("## retired-skill\ninstructions here".into());
        let picker = TaskPicker::with_skills(vec![item], "cur".into(), vec![]);
        let text = render_picker_to_text(&picker);
        assert!(
            text.contains("[retired-skill]"),
            "fallback skill tag must derive from the body's first line:\n{text}"
        );
    }

    #[test]
    fn no_skill_tag_when_session_has_none() {
        let text = render_to_text(vec![item("s1")]);
        assert!(
            !text.contains("[act] ["),
            "rows without a skill must not render a skill tag:\n{text}"
        );
    }

    #[test]
    fn skill_tag_survives_badges_and_suffix_tags() {
        // Long skill name + running badge + (current): the skill tag is fixed
        // overhead, so it must stay visible when the row overflows.
        let mut item = busy_item("s1", 1, 2);
        item.skill = Some("very-long-skill-name-that-gets-truncated".into());
        let picker = TaskPicker::with_skills(
            vec![item],
            "s1".into(),
            vec![fake_skill(
                "very-long-skill-name-that-gets-truncated",
                "very-long-skill-name-that-gets-truncated",
            )],
        );
        let text = render_picker_to_text(&picker);
        assert!(
            text.contains("[very-long-skill-n"),
            "skill tag must be visible even with badges + (current):\n{text}"
        );
        assert!(
            text.contains("\u{25cf} 1 running"),
            "running badge must survive a skill tag:\n{text}"
        );
    }

    #[test]
    fn short_preview_respects_custom_budget() {
        let preview = "x".repeat(80);
        assert_eq!(
            short_preview(&preview, 40).chars().count(),
            40,
            "39 cols + ellipsis fits 40"
        );
        assert_eq!(
            short_preview(&preview, 8).chars().count(),
            8,
            "7 cols + ellipsis fits 8"
        );
        assert_eq!(short_preview("short", 40), "short", "fits unchanged");
    }

    #[test]
    fn fork_mode_has_no_new_or_clear_rows() {
        let p = TaskPicker::new_fork(vec![item("a"), item("b")], "cur".into());
        assert_eq!(p.row_count(), 2, "sessions only, no +New / Clear all");
        assert!(
            p.clear_row_index().is_none(),
            "clear is unreachable in fork mode"
        );
        assert!(!p.confirm_clear);
    }

    #[test]
    fn fork_mode_selection_returns_fork_ids() {
        let mut p = TaskPicker::new_fork(vec![item("a"), item("b")], "cur".into());
        assert!(matches!(p.selection(), Some(TaskPick::Fork(id)) if id == "a"));
        p.move_down();
        assert!(matches!(p.selection(), Some(TaskPick::Fork(id)) if id == "b"));
        p.move_down();
        assert!(
            matches!(p.selection(), Some(TaskPick::Fork(id)) if id == "a"),
            "wraps"
        );
    }

    #[test]
    fn fork_mode_enter_returns_pick_and_closes() {
        let mut picker = Some(TaskPicker::new_fork(
            vec![item("a"), item("b")],
            "cur".into(),
        ));
        let out = handle_task_key(&mut picker, key(KeyCode::Enter));
        assert!(matches!(out, TaskOutcome::Pick(TaskPick::Fork(id)) if id == "a"));
        assert!(picker.is_none(), "picker closes after fork pick");
    }

    #[test]
    fn fork_mode_render_shows_fork_title_and_hides_aux_rows() {
        let picker = TaskPicker::new_fork(vec![item("s1")], "cur".into());
        let text = render_picker_to_text(&picker);
        assert!(text.contains("Fork"), "fork title:\n{text}");
        assert!(!text.contains("New task"), "no +New row:\n{text}");
        assert!(!text.contains("Clear all"), "no Clear-all row:\n{text}");
    }

    #[test]
    fn fork_mode_empty_sessions_enter_returns_idle() {
        let mut picker = Some(TaskPicker::new_fork(vec![], "cur".into()));
        assert_eq!(picker.as_ref().unwrap().row_count(), 0);
        let out = handle_task_key(&mut picker, key(KeyCode::Enter));
        assert!(matches!(out, TaskOutcome::Idle));
        assert!(picker.is_none(), "empty fork picker closes without a pick");
    }
}
