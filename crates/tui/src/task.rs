//! `/task` session picker - switch between or create new conversations.

use crate::task_row;
use crate::theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_store::SessionListItem;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
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
    /// `/task` - switch to / create a session ("+ New task", "Clear all" rows).
    Switch,
    /// `/fork` - pick a session to clone context from (sessions only, no
    /// "+ New task" / "Clear all" rows).
    Fork,
}

/// Modal session picker shown when the user types `/task` or `/fork`.
pub struct TaskPicker {
    sessions: Vec<SessionListItem>,
    selected: usize,
    /// Interaction mode: session switching (default) vs. fork selection.
    mode: PickerMode,
    /// The currently-active session id - always preserved by "Clear all", and
    /// tagged `(current)` in the rendered list.
    current_session_id: String,
    /// Two-step confirmation guard for the destructive "Clear all" row.
    /// `true` while we're waiting for the second Enter (or an Esc to cancel).
    confirm_clear: bool,
    /// Wall clock captured once when the picker opens; session rows show
    /// ages relative to it instead of re-rendering every second.
    now_ms: i64,
}

impl TaskPicker {
    pub fn new(sessions: Vec<SessionListItem>, current_session_id: String) -> Self {
        TaskPicker {
            sessions,
            selected: 0,
            mode: PickerMode::Switch,
            current_session_id,
            confirm_clear: false,
            now_ms: opencoder_core::message::now_ms(),
        }
    }

    /// Build a fork-mode picker (`/fork`): every listed session is a fork
    /// source, and Enter forks the highlighted session's context.
    pub fn new_fork(sessions: Vec<SessionListItem>, current_session_id: String) -> Self {
        let mut p = Self::new(sessions, current_session_id);
        p.mode = PickerMode::Fork;
        p
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

/// Fixed popup width (in display columns).
const POPUP_W: u16 = 60;

/// Display-column budget for a session row's second (preview) line: popup
/// width minus the border, highlight symbol and a little breathing room.
const PREVIEW_W: usize = 56;

/// Popup content height in terminal rows: auxiliary rows ("+ New task",
/// "Clear all") stay one line each while every session row is a two-line
/// item (age over preview), plus 4 rows of block chrome and padding.
fn popup_height(aux_rows: usize, session_count: usize) -> u16 {
    (aux_rows + session_count * 2 + 4) as u16
}

/// Render the task picker as a centered popup.
pub fn render_task_picker(f: &mut Frame, area: Rect, picker: &TaskPicker) {
    let visible = picker.row_count();
    // Session rows are two lines tall, so the popup grows by 2 rows per
    // session instead of 1.
    let aux_rows = visible - picker.sessions.len();
    let want_h = popup_height(aux_rows, picker.sessions.len())
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

    // Session rows: two lines each - the session's relative age (plus the
    // `(current)` marker) over its preview. The title is intentionally not
    // rendered; it stays available in the store for other surfaces.
    for (i, s) in picker.sessions.iter().enumerate() {
        let selected = picker.selected == i + row_offset;
        let is_current = s.id == picker.current_session_id;
        let age = task_row::relative_time(
            task_row::activity_ts(s.updated_at, s.created_at),
            picker.now_ms,
        );
        let mut headline = vec![Span::styled(age, Style::default().fg(theme::muted()))];
        if is_current {
            headline.push(Span::styled(
                "  (current)",
                Style::default().fg(theme::accent()),
            ));
        }
        let preview_style = if selected {
            Style::default()
                .fg(theme::warn_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let preview = Span::styled(task_row::preview_line(&s.preview, PREVIEW_W), preview_style);
        items.push(ListItem::new(Text::from(vec![
            Line::from(headline),
            Line::from(preview),
        ])));
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
            skill: None,
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

    // -- Two-line session row rendering -----------------------

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

    /// Fixed wall clock for the rendering tests: sessions are stamped at or
    /// near `T0` and the picker is pinned to the same instant via `now_ms`.
    const T0: i64 = 1_700_000_000_000;

    /// Build a picker whose wall clock is pinned to `now_ms` so relative
    /// ages are deterministic.
    fn picker_at(sessions: Vec<SessionListItem>, now_ms: i64) -> TaskPicker {
        let mut p = TaskPicker::new(sessions, "cur".into());
        p.now_ms = now_ms;
        p
    }

    /// Index of the first rendered line containing `needle`.
    fn line_with(text: &str, needle: &str) -> Option<usize> {
        text.lines().position(|l| l.contains(needle))
    }

    #[test]
    fn session_rows_render_two_lines_age_over_preview() {
        // updated_at is missing (0), so the age falls back to created_at.
        let mut s = item("s1");
        s.created_at = T0;
        s.preview = "hello world".into();
        let text = render_picker_to_text(&picker_at(vec![s], T0));
        let age_line = line_with(&text, "now").expect("relative age rendered");
        let preview_at = line_with(&text, "hello world").expect("preview rendered");
        assert_eq!(
            preview_at,
            age_line + 1,
            "preview must sit on its own line right under the age:\n{text}"
        );
    }

    #[test]
    fn current_marker_rides_on_the_age_line() {
        let mut s = item("cur");
        s.created_at = T0;
        let text = render_picker_to_text(&picker_at(vec![s], T0));
        let cur_line = line_with(&text, "(current)").expect("(current) rendered");
        let line = text.lines().nth(cur_line).unwrap();
        assert!(
            line.contains("now"),
            "(current) must share the age line, not the preview line:\n{text}"
        );
    }

    #[test]
    fn empty_preview_renders_placeholder_ellipsis() {
        let mut s = item("s1");
        s.created_at = T0;
        // item() leaves `preview` empty.
        let text = render_picker_to_text(&picker_at(vec![s], T0));
        let age_line = line_with(&text, "now").expect("relative age rendered");
        let preview_line = text.lines().nth(age_line + 1).expect("second row exists");
        // Strip the surrounding border cells; the row body must be just "...".
        let body = preview_line.trim().trim_matches('\u{2502}').trim();
        assert_eq!(
            body, "\u{2026}",
            "empty preview must render an ellipsis placeholder:\n{text}"
        );
    }

    #[test]
    fn popup_height_accounts_for_two_lines_per_session() {
        // aux + sessions*2 + 4: one New row + one Clear-all row + 3 sessions.
        assert_eq!(popup_height(2, 3), 12);
        // Fork mode has no aux rows.
        assert_eq!(popup_height(0, 1), 6);

        // Integration: with 2 sessions (one deletable) the popup measures
        // 2 aux + 4 session lines + 4 chrome = 10 rows tall.
        let text = render_picker_to_text(&picker_at(vec![item("cur"), item("old")], T0));
        let top = line_with(&text, "\u{256d}").expect("top border rendered");
        let bottom = line_with(&text, "\u{256f}").expect("bottom border rendered");
        assert_eq!(
            bottom - top + 1,
            popup_height(2, 2) as usize,
            "popup height must grow by 2 rows per session:\n{text}"
        );
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
