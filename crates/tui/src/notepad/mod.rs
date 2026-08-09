//! `/notepad` — IDE-style file viewer/editor for the workdir.
//!
//! Renders in a split layout: the top area shows the file-tree explorer and
//! vim editor; the bottom area shows the normal chat body + composer. A
//! draggable divider separates the two regions.
//!
//! Layout:
//! ```text
//! ┌──────────┬─────────────────────┐
//! │ Explorer │  Editor (vim)       │  ← top (height adjustable)
//! ╞════════════════════════════════╡  ← draggable divider
//! │  Chat body + composer          │  ← bottom
//! └────────────────────────────────┘
//! ```

pub mod editor;
pub mod keys;
#[cfg(test)]
mod render_tests;
pub mod search;
pub mod tree;

use std::path::PathBuf;

use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;

use crate::notepad::editor::EditorState;
use crate::notepad::search::SearchState;
use crate::notepad::tree::TreeState;

/// Which panel receives keystrokes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Editor,
}

/// Result of a key press in the notepad.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotepadOutcome {
    /// `Esc` pressed — exit the notepad and return to chat.
    Exit,
    /// Key was consumed (may or may not have changed state).
    Consumed,
    /// User pressed the focus-toggle key — switch focus to the chat composer.
    FocusChat,
}

/// Minimum bottom-area height (body + composer + status must fit).
const MIN_BOTTOM: u16 = 8;
const TREE_WIDTH: u16 = 30;

/// Top-level notepad view state.
#[derive(Clone, Debug)]
pub struct NotepadView {
    pub workdir: PathBuf,
    pub focus: Focus,
    pub tree_hidden: bool,
    /// Height of the top region (tree + editor) in terminal rows.
    /// Clamped by [`layout_split`] at render time.
    pub height: u16,
    pub tree: TreeState,
    pub editor: EditorState,
    pub search: Option<SearchState>,
}

impl NotepadView {
    pub fn new(workdir: PathBuf) -> Self {
        let (_, th) = crossterm::terminal::size().unwrap_or((80, 24));
        let height = (th * 3 / 5).max(5);
        NotepadView {
            workdir: workdir.clone(),
            focus: Focus::Tree,
            tree_hidden: false,
            height,
            tree: TreeState::new(&workdir),
            editor: EditorState::empty(),
            search: None,
        }
    }
}

/// Async wrapper called from `app.rs`. Handles one key, sets `*notepad = None`
/// when the user exits.
pub async fn dispatch_key(notepad: &mut Option<NotepadView>, k: KeyEvent) -> NotepadOutcome {
    if let Some(view) = notepad.as_mut() {
        let outcome = keys::handle_key(view, k).await;
        if outcome == NotepadOutcome::Exit {
            *notepad = None;
        }
        outcome
    } else {
        NotepadOutcome::Consumed
    }
}

// ── Rendering ──────────────────────────────────────────────────────────────

/// Render tree + editor into the given rect (the top region of the split).
pub fn render_top(f: &mut Frame, area: Rect, view: &NotepadView) {
    if let Some(s) = &view.search {
        search::render_search(f, area, s, true);
        return;
    }
    if view.tree_hidden {
        editor::render_editor(f, area, &view.editor, view.focus == Focus::Editor);
    } else {
        let hchunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(TREE_WIDTH), Constraint::Min(3)])
            .split(area);
        tree::render_tree(f, hchunks[0], &view.tree, view.focus == Focus::Tree);
        editor::render_editor(
            f,
            hchunks[1],
            &view.editor,
            view.focus == Focus::Editor,
        );
    }
}

/// Split `area` into (top, divider, bottom) rects based on the desired
/// top `height`. The divider always occupies exactly 1 row.
pub fn layout_split(area: Rect, height: u16) -> (Rect, Rect, Rect) {
    let max_height = area.height.saturating_sub(MIN_BOTTOM + 1); // +1 for divider
    let clamped = height.clamp(5, max_height.max(5));
    let vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(clamped),
            Constraint::Length(1),
            Constraint::Min(MIN_BOTTOM),
        ])
        .split(area);
    (vchunks[0], vchunks[1], vchunks[2])
}

/// Draw the draggable divider line.
pub fn render_divider(f: &mut Frame, area: Rect) {
    f.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(crate::theme::muted())),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_view(dir: &std::path::Path) -> NotepadView {
        NotepadView::new(dir.to_path_buf())
    }

    #[test]
    fn new_starts_in_tree_focus() {
        let d = tempfile::tempdir().unwrap();
        let v = make_view(d.path());
        assert_eq!(v.focus, Focus::Tree);
    }

    #[test]
    fn new_has_nonzero_height() {
        let d = tempfile::tempdir().unwrap();
        let v = make_view(d.path());
        assert!(v.height >= 5);
    }

    #[tokio::test]
    async fn dispatch_exit_clears_view() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let outcome = dispatch_key(
            &mut np,
            KeyEvent::new(crossterm::event::KeyCode::Esc, crossterm::event::KeyModifiers::NONE),
        )
        .await;
        assert_eq!(outcome, NotepadOutcome::Exit);
        assert!(np.is_none());
    }

    #[tokio::test]
    async fn dispatch_tab_cycles_focus() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), "hello").unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        // Tree -> Editor (open file to put editor in Normal mode).
        dispatch_key(
            &mut np,
            KeyEvent::new(crossterm::event::KeyCode::Enter, crossterm::event::KeyModifiers::NONE),
        )
        .await;
        assert_eq!(np.as_ref().unwrap().focus, Focus::Editor);
        // Editor -> Tree (Tab in Normal mode cycles).
        dispatch_key(
            &mut np,
            KeyEvent::new(crossterm::event::KeyCode::Tab, crossterm::event::KeyModifiers::NONE),
        )
        .await;
        assert_eq!(np.as_ref().unwrap().focus, Focus::Tree);
        // Tree -> Editor.
        dispatch_key(
            &mut np,
            KeyEvent::new(crossterm::event::KeyCode::Tab, crossterm::event::KeyModifiers::NONE),
        )
        .await;
        assert_eq!(np.as_ref().unwrap().focus, Focus::Editor);
    }

    #[tokio::test]
    async fn dispatch_enter_opens_file() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), "hello").unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        dispatch_key(
            &mut np,
            KeyEvent::new(crossterm::event::KeyCode::Enter, crossterm::event::KeyModifiers::NONE),
        )
        .await;
        let v = np.as_ref().unwrap();
        assert_eq!(v.focus, Focus::Editor);
        assert!(v.editor.file_path.is_some());
    }

    // ── layout_split ───────────────────────────────────────────────────────

    #[test]
    fn layout_split_normal() {
        let area = Rect::new(0, 0, 80, 24);
        let (top, div, bot) = layout_split(area, 15);
        assert_eq!(top.height, 15);
        assert_eq!(div.height, 1);
        assert_eq!(top.y, 0);
        assert_eq!(div.y, 15);
        assert_eq!(bot.y, 16);
        assert_eq!(bot.height, 8);
    }

    #[test]
    fn layout_split_clamps_too_large() {
        let area = Rect::new(0, 0, 80, 24);
        let (top, _, bot) = layout_split(area, 100);
        assert!(top.height < 100);
        assert!(bot.height >= MIN_BOTTOM);
    }

    #[test]
    fn layout_split_clamps_too_small() {
        let area = Rect::new(0, 0, 80, 24);
        let (top, _, _) = layout_split(area, 0);
        assert_eq!(top.height, 5);
    }

    #[test]
    fn layout_split_tiny_terminal() {
        let area = Rect::new(0, 0, 80, 12);
        let (top, div, bot) = layout_split(area, 10);
        // On a tiny terminal the function must not panic and the divider
        // is always exactly 1 row.
        assert_eq!(div.height, 1);
        let _ = (top, bot);
    }

    #[test]
    fn layout_split_divider_is_one_row() {
        let area = Rect::new(0, 0, 80, 24);
        let (_, div, _) = layout_split(area, 10);
        assert_eq!(div.height, 1);
    }
}
