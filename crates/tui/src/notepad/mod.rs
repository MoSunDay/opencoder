//! `/notepad` — IDE-style file viewer/editor for the workdir.
//!
//! Renders fullscreen: the file-tree explorer and vim editor occupy the
//! entire terminal; the chat body/composer are hidden while it is open.
//! `Esc` exits back to the normal chat view. Notepad is a pure file
//! viewer/editor — there is no way to summon the chat input while open.
//!
//! Layout:
//! ```text
//! ┌──────────┬─────────────────────┐
//! │ Explorer │  Editor (vim)       │  ← whole terminal
//! └──────────┴─────────────────────┘
//! ```

pub mod editor;
mod editor_layout;
pub mod keys;
#[cfg(test)]
mod render_tests;
pub mod search;
pub mod tree;

use std::path::PathBuf;

use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
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
}

const TREE_WIDTH: u16 = 30;

/// Top-level notepad view state.
#[derive(Clone, Debug)]
pub struct NotepadView {
    pub workdir: PathBuf,
    pub focus: Focus,
    pub tree_hidden: bool,
    pub tree: TreeState,
    pub editor: EditorState,
    pub search: Option<SearchState>,
}

impl NotepadView {
    pub fn new(workdir: PathBuf) -> Self {
        NotepadView {
            workdir: workdir.clone(),
            focus: Focus::Tree,
            tree_hidden: false,
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
    let editor_area = editor_area(area, view.tree_hidden);
    if view.tree_hidden {
        editor::render_editor(f, editor_area, &view.editor, view.focus == Focus::Editor);
    } else {
        let hchunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(TREE_WIDTH), Constraint::Min(3)])
            .split(area);
        tree::render_tree(f, hchunks[0], &view.tree, view.focus == Focus::Tree);
        editor::render_editor(f, editor_area, &view.editor, view.focus == Focus::Editor);
    }
}

/// Resolve the editor panel rectangle from the same fullscreen layout used by
/// both rendering and key handling.
pub(crate) fn editor_area(area: Rect, tree_hidden: bool) -> Rect {
    if tree_hidden {
        area
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(TREE_WIDTH), Constraint::Min(3)])
            .split(area)[1]
    }
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

    #[tokio::test]
    async fn dispatch_exit_clears_view() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let outcome = dispatch_key(
            &mut np,
            KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ),
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
            KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
        )
        .await;
        assert_eq!(np.as_ref().unwrap().focus, Focus::Editor);
        // Editor -> Tree (Tab in Normal mode cycles).
        dispatch_key(
            &mut np,
            KeyEvent::new(
                crossterm::event::KeyCode::Tab,
                crossterm::event::KeyModifiers::NONE,
            ),
        )
        .await;
        assert_eq!(np.as_ref().unwrap().focus, Focus::Tree);
        // Tree -> Editor.
        dispatch_key(
            &mut np,
            KeyEvent::new(
                crossterm::event::KeyCode::Tab,
                crossterm::event::KeyModifiers::NONE,
            ),
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
            KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
        )
        .await;
        let v = np.as_ref().unwrap();
        assert_eq!(v.focus, Focus::Editor);
        assert!(v.editor.file_path.is_some());
    }
}
