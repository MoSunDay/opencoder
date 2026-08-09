//! Integration tests for the notepad search overlay, tree-hide toggle, and
//! console command execution. Drives keys through the public
//! `notepad::keys::handle_key` entry point.
//!
//! Test-pyramid layer 2: cross-module, filesystem side-effects, no LLM.

use std::fs;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use opencoder_tui::notepad::keys::handle_key;
use opencoder_tui::notepad::{Focus, NotepadOutcome, NotepadView};

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
async fn tree_hide_toggle_navigates_through_console() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "hello").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    assert!(!v.tree_hidden);
    // Open file so the editor is in Normal mode for Tab navigation.
    press(&mut v, KeyCode::Enter).await;
    press(&mut v, KeyCode::Tab).await; // Editor -> Console
    press(&mut v, KeyCode::Esc).await; // Insert -> Normal (Tab cycles out)
    press(&mut v, KeyCode::Tab).await; // Console -> Tree
    assert_eq!(v.focus, Focus::Tree);
    // Hide tree (only works from the Tree panel).
    press(&mut v, KeyCode::Char('H')).await;
    assert!(v.tree_hidden);
    assert_eq!(v.focus, Focus::Editor);
    // Navigate back to Tree to show it again.
    press(&mut v, KeyCode::Tab).await; // Editor -> Console
    press(&mut v, KeyCode::Esc).await; // Insert -> Normal (Tab cycles out)
    press(&mut v, KeyCode::Tab).await; // Console -> Tree
    press(&mut v, KeyCode::Char('H')).await;
    assert!(!v.tree_hidden);
    assert_eq!(v.focus, Focus::Tree);
}

#[tokio::test]
async fn console_submit_bash_command() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "y").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    // Open file (Normal), then Tab to Console (starts in Insert mode).
    press(&mut v, KeyCode::Enter).await;
    press(&mut v, KeyCode::Tab).await;
    assert_eq!(v.focus, Focus::Console);
    // Type a bash command in Insert mode.
    type_str(&mut v, "!echo hello").await;
    // Esc to Normal, then Enter to submit.
    press(&mut v, KeyCode::Esc).await;
    let outcome = handle_key(&mut v, key(KeyCode::Enter)).await;
    assert_eq!(outcome, NotepadOutcome::RunBash("echo hello".into()));
    assert!(v
        .console
        .echo
        .lines
        .iter()
        .any(|l| l.text.contains("hello")));
}

#[tokio::test]
async fn console_unsubmitted_text_stays() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "y").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    // Open file (Normal), then Tab to Console (Insert mode).
    press(&mut v, KeyCode::Enter).await;
    press(&mut v, KeyCode::Tab).await;
    assert_eq!(v.focus, Focus::Console);
    // Type but do NOT submit.
    type_str(&mut v, "echo nope").await;
    assert_eq!(v.console.vim.text, "echo nope");
}
