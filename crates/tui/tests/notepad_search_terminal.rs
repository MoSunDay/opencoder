//! Integration tests for the notepad search overlay and tree-hide toggle.
//! Drives keys through the public `notepad::keys::handle_key` entry point.
//!
//! Test-pyramid layer 2: cross-module, filesystem side-effects, no LLM.

use std::fs;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use opencoder_tui::notepad::keys::handle_key;
use opencoder_tui::notepad::{Focus, NotepadView};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

async fn press(view: &mut NotepadView, code: KeyCode) {
    handle_key(view, key(code)).await;
}

async fn type_str(view: &mut NotepadView, s: &str) {
    for c in s.chars() {
        handle_key(view, key(KeyCode::Char(c))).await;
    }
}

#[tokio::test]
async fn search_open_loads_file() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "hello\nworld").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    // Open search overlay.
    press(&mut v, KeyCode::Char('/')).await;
    assert!(v.search.is_some());
    type_str(&mut v, "hello").await;
    // Execute search.
    press(&mut v, KeyCode::Enter).await;
    let s = v.search.as_ref().unwrap();
    assert!(!s.results.is_empty());
    assert!(!s.editing);
    // Open first hit.
    press(&mut v, KeyCode::Enter).await;
    assert_eq!(v.editor.vim.text, "hello\nworld");
    assert_eq!(v.focus, Focus::Editor);
    assert!(v.search.is_none());
}

#[tokio::test]
async fn search_no_match_stays_in_nav() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "hello").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    press(&mut v, KeyCode::Char('/')).await;
    type_str(&mut v, "zzznomatch").await;
    press(&mut v, KeyCode::Enter).await;
    let s = v.search.as_ref().unwrap();
    assert!(s.results.is_empty());
    assert!(!s.editing);
}

#[tokio::test]
async fn tree_hide_toggle_cycles_panels() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "y").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    press(&mut v, KeyCode::Enter).await;
    assert_eq!(v.focus, Focus::Editor);
    // Editor -> Tree (two-panel cycle).
    press(&mut v, KeyCode::Tab).await;
    assert_eq!(v.focus, Focus::Tree);
    // Hide tree (only works from the Tree panel).
    press(&mut v, KeyCode::Char('H')).await;
    assert!(v.tree_hidden);
    assert_eq!(v.focus, Focus::Editor);
    // Navigate back to Tree to show it again.
    press(&mut v, KeyCode::Tab).await;
    assert_eq!(v.focus, Focus::Tree);
    press(&mut v, KeyCode::Char('H')).await;
    assert!(!v.tree_hidden);
    assert_eq!(v.focus, Focus::Tree);
}
