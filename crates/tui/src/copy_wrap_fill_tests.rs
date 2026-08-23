//! Plan-fill integration tests: the three copy-mode clean renderers must
//! write soft-wrap flags into the shared `WrapPlan` (and respect area
//! offsets / width-mismatch guards). Driven through `TestBackend`, where
//! `render()`'s backend downcast yields `None` — the plan is passed
//! directly to the renderers instead.

use std::cell::RefCell;
use std::rc::Rc;

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::copy_wrap::WrapPlan;

// ── Plan fill through the copy-mode renderers ───────────────────────────

fn active_plan(width: u16) -> Rc<RefCell<WrapPlan>> {
    Rc::new(RefCell::new(WrapPlan {
        active: true,
        term_width: width,
        soft: Vec::new(),
    }))
}

#[test]
fn render_clean_fills_soft_for_wrapped_transcript() {
    use opencoder_session::SessionEvent;
    let mut view = crate::chat::ChatView::default();
    // 100 chars at width 40 -> one logical line, 3 visual rows.
    view.apply(&SessionEvent::TextDelta("x".repeat(100)));
    view.apply(&SessionEvent::Done);

    let plan = active_plan(40);
    let mut viewport = None;
    let mut scroll = 0u32;
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    terminal
        .draw(|f| {
            crate::copy_mode::render_clean(
                f,
                f.area(),
                &view,
                &mut scroll,
                true,
                0,
                0,
                &mut viewport,
                Some(&plan),
            )
        })
        .unwrap();
    let wp = plan.borrow();
    assert_eq!(
        &wp.soft[..3],
        &[false, true, true],
        "wrapped rows must be soft"
    );
    assert!(
        !wp.soft[3..].iter().any(|&s| s),
        "rows past the content must stay hard"
    );
}

#[test]
fn render_clean_fills_hard_for_short_lines() {
    use opencoder_session::SessionEvent;
    let mut view = crate::chat::ChatView::default();
    view.apply(&SessionEvent::TextDelta("short\nlines\nhere".into()));
    view.apply(&SessionEvent::Done);

    let plan = active_plan(40);
    let mut viewport = None;
    let mut scroll = 0u32;
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    terminal
        .draw(|f| {
            crate::copy_mode::render_clean(
                f,
                f.area(),
                &view,
                &mut scroll,
                true,
                0,
                0,
                &mut viewport,
                Some(&plan),
            )
        })
        .unwrap();
    let wp = plan.borrow();
    assert!(
        wp.soft.iter().all(|&s| !s),
        "short lines must all be hard: {:?}",
        wp.soft
    );
}

#[test]
fn render_composer_clean_fills_soft() {
    let plan = active_plan(40);
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    terminal
        .draw(|f| {
            crate::copy_mode::render_composer_clean(
                f,
                f.area(),
                &"ab".repeat(60),
                false,
                Some(&plan),
            )
        })
        .unwrap();
    let wp = plan.borrow();
    assert_eq!(
        &wp.soft[..3],
        &[false, true, true],
        "composer wrap must be soft"
    );

    // Hard newlines stay hard.
    let plan2 = active_plan(40);
    terminal
        .draw(|f| {
            crate::copy_mode::render_composer_clean(
                f,
                f.area(),
                "abc\ndef\nghi",
                false,
                Some(&plan2),
            )
        })
        .unwrap();
    let wp = plan2.borrow();
    assert!(wp.soft.iter().all(|&s| !s), "hard breaks: {:?}", wp.soft);
}

#[test]
fn render_notepad_clean_fills_soft() {
    let dir = tempfile::tempdir().unwrap();
    let long = "n".repeat(100);
    std::fs::write(dir.path().join("a.txt"), format!("{long}\nshort\n")).unwrap();
    let mut view = crate::notepad::NotepadView::new(dir.path().to_path_buf());
    view.editor.load(&dir.path().join("a.txt"));

    let plan = active_plan(40);
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    terminal
        .draw(|f| crate::copy_mode::render_notepad_clean(f, f.area(), &view, Some(&plan)))
        .unwrap();
    let wp = plan.borrow();
    // Row 0 hard, rows 1-2 continuation, row 3 hard (real newline).
    assert_eq!(
        &wp.soft[..4],
        &[false, true, true, false],
        "notepad wrap flags"
    );
    assert!(!wp.soft[4..].iter().any(|&s| s), "tail must stay hard");
}

#[test]
fn render_clean_respects_area_offset_and_width_mismatch() {
    use opencoder_session::SessionEvent;
    let mut view = crate::chat::ChatView::default();
    view.apply(&SessionEvent::TextDelta("x".repeat(100)));
    view.apply(&SessionEvent::Done);

    // Width mismatch: the plan width differs from the render width, so the
    // renderer must not fill flags (wrap columns would disagree).
    let plan = active_plan(80);
    let mut viewport = None;
    let mut scroll = 0u32;
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    terminal
        .draw(|f| {
            crate::copy_mode::render_clean(
                f,
                f.area(),
                &view,
                &mut scroll,
                true,
                0,
                0,
                &mut viewport,
                Some(&plan),
            )
        })
        .unwrap();
    assert!(
        plan.borrow().soft.is_empty(),
        "width mismatch must not fill flags"
    );
}
