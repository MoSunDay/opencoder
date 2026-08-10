//! Integration tests for the notepad editor: edit/save (`:w`), write-quit
//! (`:wq`), discard (`:q!`), and the Tab focus cycle across all three panels.
//! Drives keys through the public `notepad::keys::handle_key` entry point.
//!
//! Test-pyramid layer 2: cross-module, filesystem side-effects, no LLM.

use std::fs;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use opencoder_tui::notepad::keys::handle_key;
use opencoder_tui::notepad::{Focus, NotepadView};
use opencoder_tui::vim::VimMode;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

async fn press(view: &mut NotepadView, code: KeyCode) {
    handle_key(view, key(code)).await;
}

async fn press_ctrl(view: &mut NotepadView, code: KeyCode) {
    handle_key(view, KeyEvent::new(code, KeyModifiers::CONTROL)).await;
}

async fn type_str(view: &mut NotepadView, s: &str) {
    for c in s.chars() {
        handle_key(view, key(KeyCode::Char(c))).await;
    }
}

#[tokio::test]
async fn edit_and_save_with_colon_w() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "hello").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    press(&mut v, KeyCode::Enter).await;
    // Insert text.
    press(&mut v, KeyCode::Char('i')).await;
    type_str(&mut v, "X").await;
    press(&mut v, KeyCode::Esc).await;
    // :w (notepad intercepts before vim engine).
    press(&mut v, KeyCode::Char(':')).await;
    type_str(&mut v, "w").await;
    press(&mut v, KeyCode::Enter).await;
    assert_eq!(
        fs::read_to_string(d.path().join("a.txt")).unwrap(),
        "Xhello"
    );
    assert!(!v.editor.is_modified());
    assert_eq!(v.focus, Focus::Editor);
    assert_eq!(v.editor.vim.mode, VimMode::Normal);
}

#[tokio::test]
async fn edit_and_wq_saves_and_focuses_tree() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "hello").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    press(&mut v, KeyCode::Enter).await;
    press(&mut v, KeyCode::Char('i')).await;
    type_str(&mut v, "X").await;
    press(&mut v, KeyCode::Esc).await;
    press(&mut v, KeyCode::Char(':')).await;
    type_str(&mut v, "wq").await;
    press(&mut v, KeyCode::Enter).await;
    assert_eq!(
        fs::read_to_string(d.path().join("a.txt")).unwrap(),
        "Xhello"
    );
    assert_eq!(v.focus, Focus::Tree);
}

#[tokio::test]
async fn discard_with_colon_q_bang() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "hello").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    press(&mut v, KeyCode::Enter).await;
    press(&mut v, KeyCode::Char('i')).await;
    type_str(&mut v, "X").await;
    press(&mut v, KeyCode::Esc).await;
    press(&mut v, KeyCode::Char(':')).await;
    type_str(&mut v, "q!").await;
    press(&mut v, KeyCode::Enter).await;
    // Disk unchanged.
    assert_eq!(fs::read_to_string(d.path().join("a.txt")).unwrap(), "hello");
    // Editor text restored to original.
    assert_eq!(v.editor.vim.text, "hello");
    assert_eq!(v.focus, Focus::Tree);
}

#[tokio::test]
async fn unmodified_quit_keeps_file() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "hello").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    press(&mut v, KeyCode::Enter).await;
    // No edit — just quit.
    press(&mut v, KeyCode::Char(':')).await;
    type_str(&mut v, "q!").await;
    press(&mut v, KeyCode::Enter).await;
    assert_eq!(v.focus, Focus::Tree);
    assert_eq!(fs::read_to_string(d.path().join("a.txt")).unwrap(), "hello");
}

#[tokio::test]
async fn focus_cycle_two_panels() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "y").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    // Open file so the editor is in Normal mode (Tab cycling needs Normal).
    press(&mut v, KeyCode::Enter).await;
    assert_eq!(v.focus, Focus::Editor);
    // Editor -> Tree -> Editor (two-panel cycle, no console).
    press(&mut v, KeyCode::Tab).await;
    assert_eq!(v.focus, Focus::Tree);
    press(&mut v, KeyCode::Tab).await;
    assert_eq!(v.focus, Focus::Editor);
}

#[tokio::test]
async fn insert_session_undo_redo_restores_text() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "hello").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    press(&mut v, KeyCode::Enter).await; // open file (Normal, cursor 0)
    press(&mut v, KeyCode::Char('i')).await;
    type_str(&mut v, "XY").await;
    press(&mut v, KeyCode::Esc).await;
    assert_eq!(v.editor.vim.text, "XYhello");
    // One `u` reverts the whole insert session.
    press(&mut v, KeyCode::Char('u')).await;
    assert_eq!(v.editor.vim.text, "hello");
    // Ctrl+R redoes it.
    press_ctrl(&mut v, KeyCode::Char('r')).await;
    assert_eq!(v.editor.vim.text, "XYhello");
}

#[tokio::test]
async fn normal_mode_x_undo_redo() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "hello").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    press(&mut v, KeyCode::Enter).await;
    press(&mut v, KeyCode::Char('x')).await;
    assert_eq!(v.editor.vim.text, "ello");
    press(&mut v, KeyCode::Char('u')).await;
    assert_eq!(v.editor.vim.text, "hello");
    press_ctrl(&mut v, KeyCode::Char('r')).await;
    assert_eq!(v.editor.vim.text, "ello");
}
