//! Integration tests for editor scrolling and `:e` open-file command.
//! Test-pyramid layer 2: drives the full key dispatch through
//! `notepad::keys::handle_key` against a real tempdir.

use std::fs;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use opencoder_tui::notepad::keys::handle_key;
use opencoder_tui::notepad::{Focus, NotepadView};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn tall_content(n: usize) -> String {
    (1..=n)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

async fn open_first_file(view: &mut NotepadView) {
    // The tree's initial selection is the first file; Enter opens it.
    handle_key(view, key(KeyCode::Enter)).await;
}

#[tokio::test]
async fn big_g_advances_scroll() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("tall.txt"), tall_content(60)).unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    open_first_file(&mut v).await;
    assert_eq!(v.focus, Focus::Editor);
    assert_eq!(v.editor.scroll, 0, "scroll should start at 0");
    // Press G to go to last line
    handle_key(&mut v, key(KeyCode::Char('G'))).await;
    assert!(
        v.editor.scroll > 0,
        "scroll should advance after G, got {}",
        v.editor.scroll
    );
}

#[tokio::test]
async fn gg_resets_scroll_to_zero() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("tall.txt"), tall_content(60)).unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    open_first_file(&mut v).await;
    // Go to bottom
    handle_key(&mut v, key(KeyCode::Char('G'))).await;
    assert!(v.editor.scroll > 0);
    // Go back to top with gg
    handle_key(&mut v, key(KeyCode::Char('g'))).await;
    handle_key(&mut v, key(KeyCode::Char('g'))).await;
    assert_eq!(v.editor.scroll, 0);
}

#[tokio::test]
async fn j_advances_scroll_incrementally() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("tall.txt"), tall_content(40)).unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    open_first_file(&mut v).await;
    assert_eq!(v.editor.scroll, 0);
    // Press j many times — scroll should eventually advance
    for _ in 0..30 {
        handle_key(&mut v, key(KeyCode::Char('j'))).await;
    }
    assert!(
        v.editor.scroll > 0,
        "scroll should advance after many j presses"
    );
}

#[tokio::test]
async fn ctrl_d_moves_cursor_down() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("tall.txt"), tall_content(60)).unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    open_first_file(&mut v).await;
    let line_before = v.editor.cursor_line();
    handle_key(&mut v, ctrl('d')).await;
    let line_after = v.editor.cursor_line();
    assert!(
        line_after > line_before,
        "Ctrl-D should move cursor down: {} -> {}",
        line_before,
        line_after
    );
}

#[tokio::test]
async fn ctrl_u_moves_cursor_up() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("tall.txt"), tall_content(60)).unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    open_first_file(&mut v).await;
    // Move down first
    handle_key(&mut v, key(KeyCode::Char('G'))).await;
    let line_after_g = v.editor.cursor_line();
    assert!(line_after_g > 0);
    // Ctrl-U should move up
    handle_key(&mut v, ctrl('u')).await;
    let line_after_u = v.editor.cursor_line();
    assert!(
        line_after_u < line_after_g,
        "Ctrl-U should move cursor up: {} -> {}",
        line_after_g,
        line_after_u
    );
}

#[tokio::test]
async fn ctrl_f_full_page_down() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("tall.txt"), tall_content(60)).unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    open_first_file(&mut v).await;
    let line_before = v.editor.cursor_line();
    handle_key(&mut v, ctrl('f')).await;
    let line_after = v.editor.cursor_line();
    // Full page should move more than a few lines
    assert!(
        line_after - line_before >= 3,
        "Ctrl-F should move a full page: {} -> {}",
        line_before,
        line_after
    );
}

#[tokio::test]
async fn edit_command_opens_file() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "content a").unwrap();
    fs::write(d.path().join("b.txt"), "content b").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    open_first_file(&mut v).await;
    assert!(v
        .editor
        .file_path
        .as_ref()
        .unwrap()
        .to_string_lossy()
        .ends_with("a.txt"));
    // Type :e b.txt
    handle_key(&mut v, key(KeyCode::Char(':'))).await;
    for c in "e b.txt".chars() {
        handle_key(&mut v, key(KeyCode::Char(c))).await;
    }
    handle_key(&mut v, key(KeyCode::Enter)).await;
    assert!(v
        .editor
        .file_path
        .as_ref()
        .unwrap()
        .to_string_lossy()
        .ends_with("b.txt"));
    assert_eq!(v.editor.vim.text, "content b");
    // Focus should remain on Editor
    assert_eq!(v.focus, Focus::Editor);
}

#[tokio::test]
async fn edit_command_opens_with_edit_keyword() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "aaa").unwrap();
    fs::write(d.path().join("c.rs"), "fn main() {}").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    open_first_file(&mut v).await;
    handle_key(&mut v, key(KeyCode::Char(':'))).await;
    for c in "edit c.rs".chars() {
        handle_key(&mut v, key(KeyCode::Char(c))).await;
    }
    handle_key(&mut v, key(KeyCode::Enter)).await;
    assert_eq!(v.editor.vim.text, "fn main() {}");
}

#[tokio::test]
async fn page_scroll_does_not_fire_in_insert_mode() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("tall.txt"), tall_content(60)).unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    open_first_file(&mut v).await;
    // Enter Insert mode
    handle_key(&mut v, key(KeyCode::Char('i'))).await;
    let line_before = v.editor.cursor_line();
    // Ctrl-D in insert mode should NOT page-scroll (it's a no-op / dedent)
    handle_key(&mut v, ctrl('d')).await;
    let line_after = v.editor.cursor_line();
    assert_eq!(
        line_before, line_after,
        "Ctrl-D should not page-scroll in Insert mode"
    );
}
