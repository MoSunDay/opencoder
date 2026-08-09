//! `/notepad` — IDE-style file viewer/editor for the workdir.
//!
//! Full-screen takeover with three panels: file-tree explorer (left), vim
//! editor (right), and a vim-style console (bottom). File-content search
//! via `rg`/`grep` is available from the tree panel.
//!
//! Layout:
//! ```text
//! ┌──────────┬─────────────────────┐
//! │ Explorer │  Editor (vim)       │
//! ├──────────┴─────────────────────┤
//! │ Console (echo + composer)      │
//! └────────────────────────────────┘
//! ```

pub mod console;
pub mod editor;
pub mod keys;
#[cfg(test)]
mod render_tests;
pub mod search;
pub mod terminal;
pub mod tree;

use std::path::PathBuf;

use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::Frame;

use crate::notepad::console::ConsoleState;
use crate::notepad::editor::EditorState;
use crate::notepad::search::SearchState;
use crate::notepad::tree::TreeState;

/// Which panel receives keystrokes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Editor,
    Console,
}

/// Result of a key press in the notepad.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotepadOutcome {
    /// `Esc` pressed — exit the notepad and return to chat.
    Exit,
    /// Key was consumed (may or may not have changed state).
    Consumed,
    /// Submit `text` as a prompt to the agent session.
    SubmitPrompt(String),
    /// Run `cmd` as a background bash command.
    RunBash(String),
}

/// Top-level notepad view state.
#[derive(Clone, Debug)]
pub struct NotepadView {
    pub workdir: PathBuf,
    pub focus: Focus,
    pub tree_hidden: bool,
    pub console_hidden: bool,
    pub tree: TreeState,
    pub editor: EditorState,
    pub console: ConsoleState,
    pub search: Option<SearchState>,
}

impl NotepadView {
    pub fn new(workdir: PathBuf) -> Self {
        Self {
            tree: TreeState::new(&workdir),
            editor: EditorState::empty(),
            console: ConsoleState::new(),
            search: None,
            workdir,
            focus: Focus::Tree,
            tree_hidden: false,
            console_hidden: false,
        }
    }
}

/// Async wrapper called from `app.rs`. Handles one key, sets `*notepad = None`
/// when the user exits. Returns the outcome so the caller can act on
/// prompt submissions or bash commands.
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

const TREE_WIDTH: u16 = 30;
const CONSOLE_MIN_H: u16 = 8;

/// Full-screen render entry point.
pub fn render_frame(terminal: &mut crate::render::Term, view: &NotepadView) -> anyhow::Result<()> {
    terminal.draw(|f| {
        let area = f.area();
        let term_h = area.height;

        let (top_area, bottom_area) = if view.console_hidden {
            (area, Rect::ZERO)
        } else {
            let bottom_h = (term_h / 3)
                .max(CONSOLE_MIN_H)
                .min(term_h.saturating_sub(6));
            let vchunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(bottom_h)])
                .split(area);
            (vchunks[0], vchunks[1])
        };

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

        // Console panel (bottom).
        if !view.console_hidden {
            console::render::render_console(
                f,
                bottom_area,
                &view.console,
                view.focus == Focus::Console,
            );
        }

        place_cursor(f, view, top_area, bottom_area);
    })?;
    Ok(())
}

fn place_cursor(_f: &mut Frame, view: &NotepadView, top: Rect, bottom: Rect) {
    match view.focus {
        Focus::Console => {
            // Cursor is set inside render_composer via set_cursor_position.
            let _ = (top, bottom);
        }
        Focus::Editor => {
            // Cursor is set inside render_editor via set_editor_cursor.
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
    fn new_has_empty_editor_and_console() {
        let v = make_view();
        assert!(v.editor.file_path.is_none());
        assert!(v.console.echo.is_empty());
        assert!(v.search.is_none());
        assert!(!v.console_hidden);
    }

    #[tokio::test]
    async fn dispatch_exit_clears_view() {
        let mut np: Option<NotepadView> = Some(make_view());
        let outcome = dispatch_key(
            &mut np,
            KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ),
        )
        .await;
        assert!(np.is_none());
        assert_eq!(outcome, NotepadOutcome::Exit);
    }

    #[tokio::test]
    async fn dispatch_tab_cycles_focus() {
        let mut np: Option<NotepadView> = Some(make_view());
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
        let mut np: Option<NotepadView> = Some(make_view());
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

    #[tokio::test]
    async fn console_submit_prompt_outcome() {
        let mut np: Option<NotepadView> = Some(make_view());
        np.as_mut().unwrap().focus = Focus::Console;
        np.as_mut().unwrap().console.vim.text = "hello agent".into();
        np.as_mut().unwrap().console.vim.mode = crate::vim::VimMode::Normal;
        let outcome = dispatch_key(
            &mut np,
            KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
        )
        .await;
        assert_eq!(outcome, NotepadOutcome::SubmitPrompt("hello agent".into()));
        // Composer should be cleared after submit.
        let v = np.as_ref().unwrap();
        assert!(v.console.vim.text.is_empty());
        assert!(!v.console.echo.is_empty());
    }

    #[tokio::test]
    async fn console_submit_bash_outcome() {
        let mut np: Option<NotepadView> = Some(make_view());
        np.as_mut().unwrap().focus = Focus::Console;
        np.as_mut().unwrap().console.vim.text = "!ls".into();
        np.as_mut().unwrap().console.vim.mode = crate::vim::VimMode::Normal;
        let outcome = dispatch_key(
            &mut np,
            KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
        )
        .await;
        assert_eq!(outcome, NotepadOutcome::RunBash("ls".into()));
    }

    #[tokio::test]
    async fn console_empty_submit_is_consumed() {
        let mut np: Option<NotepadView> = Some(make_view());
        np.as_mut().unwrap().focus = Focus::Console;
        np.as_mut().unwrap().console.vim.mode = crate::vim::VimMode::Normal;
        let outcome = dispatch_key(
            &mut np,
            KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
        )
        .await;
        assert_eq!(outcome, NotepadOutcome::Consumed);
    }
}
