//! Integration tests for the notepad file-tree lifecycle: open, expand,
//! create, delete. Drives the full key sequence through the public
//! `notepad::keys::handle_key` entry point against a real tempdir.
//!
//! Test-pyramid layer 2: cross-module, filesystem side-effects, no LLM.

use std::fs;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use opencoder_tui::notepad::keys::handle_key;
use opencoder_tui::notepad::tree::TreeInput;
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
async fn open_file_loads_into_editor() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "hello\nworld").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    assert_eq!(v.focus, Focus::Tree);
    press(&mut v, KeyCode::Enter).await;
    assert_eq!(v.editor.vim.text, "hello\nworld");
    assert_eq!(v.editor.file_path, Some(d.path().join("a.txt")));
    assert_eq!(v.focus, Focus::Editor);
}

#[tokio::test]
async fn expand_then_collapse_dir() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("sub")).unwrap();
    fs::write(d.path().join("sub").join("inner.txt"), "x").unwrap();
    fs::write(d.path().join("a.txt"), "y").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    // Dirs sort first: index 0 == "sub" (collapsed by default).
    assert!(v.tree.flat[0].is_dir);
    let count_collapsed = v.tree.flat.len();
    // Expand.
    press(&mut v, KeyCode::Enter).await;
    assert!(!v.tree.flat[0].collapsed);
    assert!(v.tree.flat.len() > count_collapsed);
    assert!(v.tree.flat.iter().any(|n| n.name == "inner.txt"));
    // Collapse.
    press(&mut v, KeyCode::Enter).await;
    assert!(v.tree.flat[0].collapsed);
    assert_eq!(v.tree.flat.len(), count_collapsed);
    assert!(!v.tree.flat.iter().any(|n| n.name == "inner.txt"));
}

#[tokio::test]
async fn open_nested_file_after_expand() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("sub")).unwrap();
    fs::write(d.path().join("sub").join("inner.txt"), "deep content").unwrap();
    fs::write(d.path().join("a.txt"), "top").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    // Expand "sub" (index 0).
    press(&mut v, KeyCode::Enter).await;
    // Move down to inner.txt.
    press(&mut v, KeyCode::Char('j')).await;
    assert_eq!(v.tree.selected_node().unwrap().name, "inner.txt");
    // Open it.
    press(&mut v, KeyCode::Enter).await;
    assert_eq!(v.editor.vim.text, "deep content");
    assert_eq!(
        v.editor.file_path,
        Some(d.path().join("sub").join("inner.txt"))
    );
    assert_eq!(v.focus, Focus::Editor);
}

#[tokio::test]
async fn create_file_flow() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "y").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    press(&mut v, KeyCode::Char('n')).await;
    assert!(v.tree.input.is_some());
    type_str(&mut v, "new.txt").await;
    press(&mut v, KeyCode::Enter).await;
    assert!(d.path().join("new.txt").exists());
    assert_eq!(fs::read_to_string(d.path().join("new.txt")).unwrap(), "");
    assert!(v.tree.flat.iter().any(|n| n.name == "new.txt"));
    assert!(v.tree.input.is_none());
}

#[tokio::test]
async fn create_file_cancelled_by_esc() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "y").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    press(&mut v, KeyCode::Char('n')).await;
    type_str(&mut v, "new.txt").await;
    press(&mut v, KeyCode::Esc).await;
    assert!(!d.path().join("new.txt").exists());
    assert!(v.tree.input.is_none());
}

#[tokio::test]
async fn delete_file_flow() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "y").unwrap();
    fs::write(d.path().join("b.txt"), "z").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    press(&mut v, KeyCode::Char('d')).await;
    assert!(matches!(
        v.tree.input,
        Some(TreeInput::DeleteConfirm { .. })
    ));
    press(&mut v, KeyCode::Char('y')).await;
    assert!(!d.path().join("a.txt").exists());
    assert!(d.path().join("b.txt").exists());
    assert!(!v.tree.flat.iter().any(|n| n.name == "a.txt"));
    assert!(v.tree.input.is_none());
}

#[tokio::test]
async fn delete_file_cancelled_by_n() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "y").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    press(&mut v, KeyCode::Char('d')).await;
    press(&mut v, KeyCode::Char('n')).await;
    assert!(d.path().join("a.txt").exists());
    assert!(v.tree.flat.iter().any(|n| n.name == "a.txt"));
    assert!(v.tree.input.is_none());
}

#[tokio::test]
async fn delete_directory_flow() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("sub")).unwrap();
    fs::write(d.path().join("sub").join("inner.txt"), "x").unwrap();
    fs::write(d.path().join("a.txt"), "y").unwrap();
    let mut v = NotepadView::new(d.path().to_path_buf());
    // "sub" is index 0.
    press(&mut v, KeyCode::Char('d')).await;
    press(&mut v, KeyCode::Char('y')).await;
    assert!(!d.path().join("sub").exists());
    assert!(!v.tree.flat.iter().any(|n| n.name == "sub"));
    assert!(v.tree.input.is_none());
}
