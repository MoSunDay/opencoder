//! Regression tests: rendering at extreme sizes and pathological inputs
//! must not panic. Exercises the lazy tree, editor cursor math, and panel
//! boundary clamping via ratatui's `TestBackend`.

use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

use crate::notepad::editor::{render_editor, EditorState};
use crate::notepad::tree::{render_tree, TreeState};
use crate::notepad::NotepadView;
use crate::vim::{VimMode, VimState};

/// Render a tree into a TestBackend terminal at the given area, asserting no panic.
fn render_tree_no_panic(area: Rect, state: &TreeState, focused: bool) {
    let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
    term.draw(|f| {
        render_tree(f, area, state, focused);
    })
    .unwrap();
}

fn render_editor_no_panic(area: Rect, state: &EditorState, focused: bool) {
    let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
    term.draw(|f| {
        render_editor(f, area, state, focused);
    })
    .unwrap();
}

// ── Tree panel ─────────────────────────────────────────────────────────────

#[test]
fn render_tree_tiny_area() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.txt"), "x").unwrap();
    let st = TreeState::new(d.path());
    render_tree_no_panic(Rect::new(0, 0, 1, 1), &st, true);
}

#[test]
fn render_tree_zero_height() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.txt"), "x").unwrap();
    let st = TreeState::new(d.path());
    // height 0 after border subtraction
    render_tree_no_panic(Rect::new(0, 0, 10, 2), &st, false);
}

#[test]
fn render_tree_zero_width() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.txt"), "x").unwrap();
    let st = TreeState::new(d.path());
    render_tree_no_panic(Rect::new(0, 0, 1, 10), &st, false);
}

#[test]
fn render_tree_large_workdir_no_crash() {
    // Pathological workdir: thousands of top-level files.
    // Lazy loading means only root is scanned — must not OOM.
    let d = tempfile::tempdir().unwrap();
    for i in 0..3000 {
        std::fs::write(d.path().join(format!("f{:04}.txt", i)), "").unwrap();
    }
    let st = TreeState::new(d.path());
    // Flatten should be capped.
    assert!(st.flat.len() <= 5001);
    render_tree_no_panic(Rect::new(0, 0, 30, 20), &st, true);
}

#[test]
fn render_tree_deep_workdir_no_crash() {
    // Pathological workdir: extremely deep nesting.
    // Lazy loading + depth cap must prevent stack overflow.
    let d = tempfile::tempdir().unwrap();
    let mut p = d.path().to_path_buf();
    for i in 0..50 {
        p = p.join(format!("d{}", i));
        std::fs::create_dir_all(&p).unwrap();
    }
    let st = TreeState::new(d.path());
    render_tree_no_panic(Rect::new(0, 0, 30, 20), &st, true);
}

// ── Editor panel ───────────────────────────────────────────────────────────

#[test]
fn render_editor_tiny_area() {
    let mut ed = EditorState::empty();
    ed.vim = VimState::new("hello".to_string());
    render_editor_no_panic(Rect::new(0, 0, 1, 1), &ed, true);
}

#[test]
fn render_editor_zero_height() {
    let mut ed = EditorState::empty();
    ed.vim = VimState::new("hello".to_string());
    render_editor_no_panic(Rect::new(0, 0, 20, 2), &ed, false);
}

#[test]
fn render_editor_long_line_cursor_no_overflow() {
    // A line wider than u16::MAX would overflow raw addition.
    // We clamp, so this must not panic.
    let mut ed = EditorState::empty();
    let long_line = "x".repeat(200);
    ed.vim = VimState::new(long_line);
    ed.vim.cursor = 150;
    ed.vim.mode = VimMode::Normal;
    render_editor_no_panic(Rect::new(0, 0, 40, 10), &ed, true);
}

#[test]
fn render_editor_many_lines_cursor_no_overflow() {
    // Many lines — cursor row math must not overflow u16.
    let mut ed = EditorState::empty();
    let text: String = (0..500)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    ed.vim = VimState::new(text);
    // Move cursor to a deep line.
    ed.vim.cursor = 2000;
    ed.vim.mode = VimMode::Normal;
    render_editor_no_panic(Rect::new(0, 0, 40, 10), &ed, true);
}

#[test]
fn render_editor_command_mode_no_cursor() {
    let mut ed = EditorState::empty();
    ed.vim = VimState::new("hi".to_string());
    ed.vim.mode = VimMode::Command;
    ed.vim.cmdline = "wq".to_string();
    render_editor_no_panic(Rect::new(0, 0, 40, 10), &ed, true);
}

#[test]
fn render_editor_empty_buffer_no_panic() {
    // Regression: an unmodified EditorState::empty() has text == "".
    // "".lines() yields zero elements, so the old `total = lines.len().max(1)`
    // forced `end = 1` and sliced `lines[0..1]` on a length-0 vec -> panic on
    // the very first `/notepad` frame. This must render without panicking and
    // still show the line-1 gutter placeholder.
    let ed = EditorState::empty();

    let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
    term.draw(|f| {
        render_editor(f, Rect::new(0, 0, 40, 10), &ed, true);
    })
    .unwrap();

    // The block border insets content by 1; the empty-buffer placeholder
    // renders " 1 " starting at the inner top-left, so row 1 must contain
    // a "1" cell within the gutter region.
    let buf = term.backend().buffer();
    let row = 1u16;
    let has_line_one = (0..6u16).any(|x| buf.cell((x, row)).map(|c| c.symbol()) == Some("1"));
    assert!(has_line_one, "empty editor should show line-1 gutter");
}

// ── Full notepad view ──────────────────────────────────────────────────────

#[test]
fn notepad_view_extreme_small_terminal() {
    // Simulate the full notepad render at 2x3 (minimum viable).
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.txt"), "hello").unwrap();
    let view = NotepadView::new(d.path().to_path_buf());

    let mut term = Terminal::new(TestBackend::new(3, 2)).unwrap();
    // Drawing through the component-level render functions directly,
    // since render_frame requires a real CrosstermBackend.
    term.draw(|f| {
        let area = f.area();
        // Just exercise tree + editor at tiny sizes.
        render_tree(
            f,
            Rect::new(0, 0, area.width, area.height),
            &view.tree,
            true,
        );
    })
    .unwrap();
}
