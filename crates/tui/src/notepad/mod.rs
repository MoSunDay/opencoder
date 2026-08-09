//! `/notepad` — IDE-style file viewer/editor for the workdir.
//!
//! Full-screen takeover with three panels: file-tree explorer (left), vim
//! editor (right), and a pseudo-terminal (bottom). File-content search via
//! `rg`/`grep` is available from the tree panel.
//!
//! Layout:
//! ```text
//! ┌──────────┬─────────────────────┐
//! │ Explorer │  Editor (vim)       │
//! ├──────────┴─────────────────────┤
//! │ Terminal (sh -c)               │
//! └────────────────────────────────┘
//! ```

pub mod editor;
pub mod keys;
pub mod search;
pub mod terminal;
#[cfg(test)]
mod render_tests;
pub mod tree;

use std::path::PathBuf;

use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::Frame;

use crate::notepad::editor::EditorState;
use crate::notepad::search::SearchState;
use crate::notepad::terminal::TerminalState;
use crate::notepad::tree::TreeState;

/// Which panel receives keystrokes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Editor,
    Terminal,
}

/// Result of a key press in the notepad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotepadOutcome {
    /// `Esc` pressed — exit the notepad and return to chat.
    Exit,
    /// Key was consumed (may or may not have changed state).
    Consumed,
}

/// Top-level notepad view state.
#[derive(Clone, Debug)]
pub struct NotepadView {
    pub workdir: PathBuf,
    pub focus: Focus,
    pub tree_hidden: bool,
    pub tree: TreeState,
    pub editor: EditorState,
    pub terminal: TerminalState,
    pub search: Option<SearchState>,
}

impl NotepadView {
    pub fn new(workdir: PathBuf) -> Self {
        Self {
            tree: TreeState::new(&workdir),
            editor: EditorState::empty(),
            terminal: TerminalState::new(),
            search: None,
            workdir,
            focus: Focus::Tree,
            tree_hidden: false,
        }
    }
}

/// Async wrapper called from `app.rs`. Handles one key, sets `*notepad = None`
/// when the user exits.
pub async fn dispatch_key(
    notepad: &mut Option<NotepadView>,
    k: KeyEvent,
    input: &mut String,
    cursor_idx: &mut usize,
) {
    if let Some(view) = notepad.as_mut() {
        let outcome = keys::handle_key(view, k, input, cursor_idx).await;
        if outcome == NotepadOutcome::Exit {
            *notepad = None;
        }
    }
}

const TREE_WIDTH: u16 = 30;
const TERM_MIN_H: u16 = 6;

/// Full-screen render entry point.
pub fn render_frame(
    terminal: &mut crate::render::Term,
    view: &NotepadView,
    input: &str,
    cursor_idx: usize,
) -> anyhow::Result<()> {
    terminal.draw(|f| {
        let area = f.area();
        let term_h = area.height;

        // Vertical split: top (tree+editor) | bottom (terminal).
        let bottom_h = (term_h / 4).max(TERM_MIN_H).min(term_h.saturating_sub(6));
        let vchunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(bottom_h)])
            .split(area);
        let top_area = vchunks[0];
        let bottom_area = vchunks[1];

        // Search overlay takes over the top area.
        if let Some(s) = &view.search {
            search::render_search(f, top_area, s, true);
        } else if view.tree_hidden {
            editor::render_editor(f, top_area, &view.editor, view.focus == Focus::Editor);
        } else {
            let hchunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(TREE_WIDTH), Constraint::Min(3)])
                .split(top_area);
            tree::render_tree(f, hchunks[0], &view.tree, view.focus == Focus::Tree);
            editor::render_editor(f, hchunks[1], &view.editor, view.focus == Focus::Editor);
        }

        // Terminal panel (always visible at the bottom).
        let term_input = if view.focus == Focus::Terminal { input } else { "" };
        terminal::render_terminal(
            f,
            bottom_area,
            &view.terminal,
            term_input,
            view.focus == Focus::Terminal,
        );

        // Hardware cursor placement.
        place_cursor(f, view, top_area, bottom_area, input, cursor_idx);

    })?;
    Ok(())
}

fn place_cursor(
    f: &mut Frame,
    view: &NotepadView,
    top: Rect,
    bottom: Rect,
    input: &str,
    _cursor_idx: usize,
) {
    match view.focus {
        Focus::Terminal => {
            // Position after "❯ " + input.
            let col = crate::composer::str_width(input).min(u16::MAX as usize) as u16;
            // border (1) + "❯ " prefix (2) + input width
            let x = bottom.x.saturating_add(1).saturating_add(2).saturating_add(col);
            // Clamp x to stay within the terminal area.
            let max_x = bottom.right().saturating_sub(1);
            let x = x.min(max_x);
            let y = bottom.y + bottom.height.saturating_sub(2);
            f.set_cursor_position((x, y));
        }
        Focus::Editor => {
            // Cursor is set inside render_editor via set_cursor_position.
            let _ = top;
        }
        Focus::Tree => {
            // Hide cursor in tree panel.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_view() -> NotepadView {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), "hello\nworld").unwrap();
        NotepadView::new(d.path().to_path_buf())
    }

    #[test]
    fn new_starts_in_tree_focus() {
        let v = make_view();
        assert_eq!(v.focus, Focus::Tree);
        assert!(!v.tree.flat.is_empty());
    }

    #[test]
    fn new_has_empty_editor_and_terminal() {
        let v = make_view();
        assert!(v.editor.file_path.is_none());
        assert!(v.terminal.lines.is_empty());
        assert!(v.search.is_none());
    }

    #[tokio::test]
    async fn dispatch_exit_clears_view() {
        let mut np: Option<NotepadView> = Some(make_view());
        let mut input = String::new();
        let mut cur = 0;
        dispatch_key(
            &mut np,
            KeyEvent::new(crossterm::event::KeyCode::Esc, crossterm::event::KeyModifiers::NONE),
            &mut input,
            &mut cur,
        )
        .await;
        assert!(np.is_none());
    }

    #[tokio::test]
    async fn dispatch_tab_cycles_focus() {
        let mut np: Option<NotepadView> = Some(make_view());
        let mut input = String::new();
        let mut cur = 0;
        dispatch_key(
            &mut np,
            KeyEvent::new(crossterm::event::KeyCode::Tab, crossterm::event::KeyModifiers::NONE),
            &mut input,
            &mut cur,
        )
        .await;
        assert_eq!(np.as_ref().unwrap().focus, Focus::Editor);
    }

    #[tokio::test]
    async fn dispatch_enter_opens_file() {
        let mut np: Option<NotepadView> = Some(make_view());
        let mut input = String::new();
        let mut cur = 0;
        dispatch_key(
            &mut np,
            KeyEvent::new(crossterm::event::KeyCode::Enter, crossterm::event::KeyModifiers::NONE),
            &mut input,
            &mut cur,
        )
        .await;
        let v = np.as_ref().unwrap();
        assert_eq!(v.focus, Focus::Editor);
        assert!(v.editor.file_path.is_some());
    }
}
