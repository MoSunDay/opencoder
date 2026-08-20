//! Picker state + key handling for the `@` file-mention menu.
//!
//! Mirrors `menu.rs::SkillMenu` / `command.rs::CommandMenu`: typed characters
//! grow the query and re-filter (fuzzy via `menu::fuzzy_score`), ↑/↓ wrap,
//! `Enter`/`Tab` pick, `Esc` closes. `handle_file_key` takes the
//! `Option<FileMenu>` slot (`take`/put-back) so `app.rs` never
//! double-borrows.

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::list::{collect_entries, FileEntry};
use crate::menu::fuzzy_score;

/// Outcome of a keystroke while the `@` menu is open. `Pick` carries the
/// ready-to-insert composer token (`@relative/path ` — the leading `@` is
/// re-emitted by the pick because the trigger keystroke was consumed, and
/// the trailing space starts the next pin at a fresh token boundary) so
/// the caller just inserts it at the cursor. The `@` marker keeps the
/// token recognizable by submit-time mention expansion.
#[derive(Debug, PartialEq, Eq)]
pub enum FileOutcome {
    /// Menu still open; nothing to do.
    Idle,
    /// Insert this token into the composer and close the menu.
    Pick(String),
    /// Menu closed without a pick.
    Close,
}

/// Picker state for the `@` file menu.
pub struct FileMenu {
    entries: Vec<FileEntry>,
    /// Visible rows (indices into `entries`, filtered + score-sorted).
    rows: Vec<usize>,
    selected: usize,
    query: String,
}

impl FileMenu {
    /// Walk `workdir` (capped, gitignore-aware) and open the picker.
    pub fn new(workdir: &Path) -> Self {
        let mut m = FileMenu {
            entries: collect_entries(workdir),
            rows: Vec::new(),
            selected: 0,
            query: String::new(),
        };
        m.refilter();
        m
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn visible_count(&self) -> usize {
        self.rows.len()
    }

    /// The entry under the highlight, if any.
    pub fn selected_entry(&self) -> Option<&FileEntry> {
        self.entries.get(*self.rows.get(self.selected)?)
    }

    /// Visible entries after filtering (render order).
    pub fn visible_entries(&self) -> impl Iterator<Item = &FileEntry> {
        self.rows.iter().map(|&i| &self.entries[i])
    }

    /// Highlighted row index within the visible list (render state).
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

    fn on_char(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    fn on_backspace(&mut self) {
        self.query.pop();
        self.refilter();
    }

    /// Rebuild `rows` (fuzzy subsequence on the path, compact matches first)
    /// and clamp the selection.
    fn refilter(&mut self) {
        let q = self.query.trim().to_lowercase();
        let mut scored: Vec<(i32, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| fuzzy_score(&q, &e.rel.to_lowercase()).map(|s| (s, i)))
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        self.rows = scored.into_iter().map(|(_, i)| i).collect();
        if self.rows.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.rows.len() - 1);
        }
    }
}

/// Handle one keystroke while the `@` file menu is open. Consumes the slot;
/// puts the menu back unless the outcome ends it (`Pick`/`Close`).
pub fn handle_file_key(slot: &mut Option<FileMenu>, k: KeyEvent) -> FileOutcome {
    // Ctrl combos other than Ctrl-D are ignored (mirrors the `$` menu);
    // Ctrl-D hard-closes as the stuck-modal escape hatch.
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(k.code, KeyCode::Char('d') | KeyCode::Char('\u{4}')) {
            *slot = None;
            return FileOutcome::Close;
        }
        return FileOutcome::Idle;
    }
    let mut menu = match slot.take() {
        Some(m) => m,
        None => return FileOutcome::Idle,
    };
    let outcome = match k.code {
        KeyCode::Up => {
            menu.move_up();
            FileOutcome::Idle
        }
        KeyCode::Down => {
            menu.move_down();
            FileOutcome::Idle
        }
        KeyCode::Backspace => {
            // Empty query + Backspace closes (IDE convention).
            if menu.query.is_empty() {
                FileOutcome::Close
            } else {
                menu.on_backspace();
                FileOutcome::Idle
            }
        }
        KeyCode::Enter | KeyCode::Tab => match menu.selected_entry() {
            Some(e) => FileOutcome::Pick(format!("@{} ", e.rel)),
            None => FileOutcome::Close,
        },
        KeyCode::Esc => FileOutcome::Close,
        KeyCode::Char(c) => {
            menu.on_char(c);
            FileOutcome::Idle
        }
        _ => FileOutcome::Idle,
    };
    if matches!(outcome, FileOutcome::Idle) {
        *slot = Some(menu);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn menu_with(rels: &[&str]) -> FileMenu {
        let mut m = FileMenu {
            entries: rels
                .iter()
                .map(|r| FileEntry {
                    rel: (*r).into(),
                    is_dir: false,
                })
                .collect(),
            rows: Vec::new(),
            selected: 0,
            query: String::new(),
        };
        m.refilter();
        m
    }

    #[test]
    fn opens_with_all_rows_and_zero_selection() {
        let m = menu_with(&["a.txt", "src/main.rs"]);
        assert_eq!(m.visible_count(), 2);
        assert_eq!(m.selected_entry().unwrap().rel, "a.txt");
    }

    #[test]
    fn typed_chars_filter_and_navigate_wraps() {
        let mut slot = Some(menu_with(&["a.txt", "main.rs", "src/main.rs"]));
        handle_file_key(&mut slot, key(KeyCode::Char('m')));
        handle_file_key(&mut slot, key(KeyCode::Char('a')));
        let m = slot.as_ref().unwrap();
        assert!(
            m.visible_count() < 3,
            "query filters: {}",
            m.visible_count()
        );
        assert!(m.selected_entry().unwrap().rel.contains("ma"));
        handle_file_key(&mut slot, key(KeyCode::Down));
        handle_file_key(&mut slot, key(KeyCode::Down));
        // Wrap: two rows -> second Down lands back on the first.
        let m = slot.as_ref().unwrap();
        assert_eq!(m.selected, 0);
    }

    #[test]
    fn enter_picks_at_prefixed_token_with_trailing_space() {
        let mut slot = Some(menu_with(&["src/main.rs"]));
        let out = handle_file_key(&mut slot, key(KeyCode::Enter));
        assert_eq!(out, FileOutcome::Pick("@src/main.rs ".to_string()));
        assert!(slot.is_none(), "pick closes the menu");
    }

    #[test]
    fn tab_picks_esc_closes_backspace_on_empty_closes() {
        let mut slot = Some(menu_with(&["a.txt"]));
        assert_eq!(
            handle_file_key(&mut slot, key(KeyCode::Tab)),
            FileOutcome::Pick("@a.txt ".to_string())
        );

        let mut slot = Some(menu_with(&["a.txt"]));
        assert_eq!(
            handle_file_key(&mut slot, key(KeyCode::Esc)),
            FileOutcome::Close
        );
        assert!(slot.is_none());

        let mut slot = Some(menu_with(&["a.txt"]));
        assert_eq!(
            handle_file_key(&mut slot, key(KeyCode::Backspace)),
            FileOutcome::Close
        );
    }

    #[test]
    fn ctrl_d_hard_closes_empty_enter_closes() {
        let mut slot = Some(menu_with(&["a.txt"]));
        assert_eq!(
            handle_file_key(
                &mut slot,
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)
            ),
            FileOutcome::Close
        );

        let mut slot = Some(menu_with(&[]));
        assert_eq!(
            handle_file_key(&mut slot, key(KeyCode::Enter)),
            FileOutcome::Close
        );
    }

    #[test]
    fn backspace_pops_query_instead_of_closing() {
        let mut slot = Some(menu_with(&["a.txt"]));
        handle_file_key(&mut slot, key(KeyCode::Char('z')));
        assert_eq!(
            handle_file_key(&mut slot, key(KeyCode::Backspace)),
            FileOutcome::Idle
        );
        assert_eq!(slot.as_ref().unwrap().query(), "");
        assert!(slot.is_some(), "stays open after popping to empty");
    }
}
