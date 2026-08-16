use super::*;
use crate::chat::ChatView;
use opencoder_session::SessionEvent;

/// When not following, the body's bottom-border row shows the `⬇` (U+2B07)
/// follow indicator and exports its hit rect via `jump_btn`.
#[test]
fn body_follow_indicator_when_not_following() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hello".into()));
    v.apply(&SessionEvent::Done);

    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut jump_btn: Option<Rect> = None;
    let mut top_btn: Option<Rect> = None;
    let mut body_out: Option<Rect> = None;
    let mut scroll = 0u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                &Line::raw("test"),
                &mut scroll,
                false,
                0,
                0,
                &mut body_out,
                &mut jump_btn,
                &mut top_btn,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut None,
                true,
                0,
                false,
            );
        })
        .unwrap();

    // Bottom border row is the last row of the area.
    let area = terminal.backend().buffer().area;
    let bottom_row = area.bottom() - 1;
    let row = row_text(terminal.backend().buffer(), bottom_row, area.width);
    assert!(
        row.contains('\u{2b07}'),
        "follow arrow ⬇ should appear on bottom border; got: {row}"
    );
    assert!(
        jump_btn.is_some(),
        "jump_btn should be set to a rect when not following"
    );
}

/// When following, the body's bottom-border row shows the "跟随中…" label
/// and `jump_btn` is `None` (no clickable target while already at the bottom).
#[test]
fn body_follow_label_when_following() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hello".into()));
    v.apply(&SessionEvent::Done);

    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut jump_btn: Option<Rect> = None;
    let mut top_btn: Option<Rect> = None;
    let mut body_out: Option<Rect> = None;
    let mut scroll = 0u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                &Line::raw("test"),
                &mut scroll,
                true,
                0,
                0,
                &mut body_out,
                &mut jump_btn,
                &mut top_btn,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut None,
                true,
                0,
                false,
            );
        })
        .unwrap();

    let area = terminal.backend().buffer().area;
    let bottom_row = area.bottom() - 1;
    let row = row_text(terminal.backend().buffer(), bottom_row, area.width);
    assert!(
        row.contains('跟') && row.contains('随'),
        "follow label should appear on bottom border; got: {row}"
    );
    assert!(jump_btn.is_none(), "jump_btn should be None when following");
}

/// When scrolled past the top, the body's top-border row shows the `⬆`
/// (U+2B06) jump-to-top indicator and exports its hit rect via `top_btn`.
/// Unlike the bottom indicator it carries no "跟随中"-style label.
#[test]
fn body_top_arrow_when_scrolled_down() {
    let mut v = ChatView::default();
    // Plenty of lines so the content overflows a 40x10 window.
    let body = (0..40).map(|i| format!("line {i}\n")).collect::<String>();
    v.apply(&SessionEvent::TextDelta(body));
    v.apply(&SessionEvent::Done);

    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut jump_btn: Option<Rect> = None;
    let mut top_btn: Option<Rect> = None;
    let mut body_out: Option<Rect> = None;
    let mut scroll = 5u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                &Line::raw("test"),
                &mut scroll,
                false,
                0,
                0,
                &mut body_out,
                &mut jump_btn,
                &mut top_btn,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut None,
                true,
                0,
                false,
            );
        })
        .unwrap();

    let area = terminal.backend().buffer().area;
    let top_row = row_text(terminal.backend().buffer(), area.y, area.width);
    assert!(
        top_row.contains('\u{2b06}'),
        "jump-to-top arrow ⬆ should appear on top border; got: {top_row}"
    );
    assert!(
        top_btn.is_some(),
        "top_btn should be set to a rect when scrolled past the top"
    );
}

/// At the very top (scroll 0) no jump-to-top arrow is shown and `top_btn` is
/// `None` — nothing to scroll up to.
#[test]
fn body_no_top_arrow_when_at_top() {
    let mut v = ChatView::default();
    let body = (0..40).map(|i| format!("line {i}\n")).collect::<String>();
    v.apply(&SessionEvent::TextDelta(body));
    v.apply(&SessionEvent::Done);

    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut jump_btn: Option<Rect> = None;
    let mut top_btn: Option<Rect> = None;
    let mut body_out: Option<Rect> = None;
    let mut scroll = 0u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                &Line::raw("test"),
                &mut scroll,
                false,
                0,
                0,
                &mut body_out,
                &mut jump_btn,
                &mut top_btn,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut None,
                true,
                0,
                false,
            );
        })
        .unwrap();

    let area = terminal.backend().buffer().area;
    let top_row = row_text(terminal.backend().buffer(), area.y, area.width);
    assert!(
        !top_row.contains('\u{2b06}'),
        "no jump-to-top arrow when at the top; got: {top_row}"
    );
    assert!(top_btn.is_none(), "top_btn should be None when at the top");
}

// ----- Tutorial: empty-session welcome text shows & auto-hides -----

/// An empty session renders the in-body tutorial (`render_tutorial_in_body`)
/// whose first content line includes the "OpenCoder" brand token. As soon as
/// the first block appears the tutorial disappears. This covers the
/// `chat.blocks.is_empty()` early-return branch in `render_body`.
#[test]
fn empty_session_shows_tutorial_then_hides_on_first_block() {
    let backend = TestBackend::new(60, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let area = terminal.backend().buffer().area;

    // --- State 1: empty session → tutorial visible ---
    let v = ChatView::default();
    let mut jump_btn: Option<Rect> = None;
    let mut top_btn: Option<Rect> = None;
    let mut body_out: Option<Rect> = None;
    let mut scroll = 0u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                &Line::raw("test"),
                &mut scroll,
                false,
                0,
                0,
                &mut body_out,
                &mut jump_btn,
                &mut top_btn,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut None,
                true,
                0,
                false,
            );
        })
        .unwrap();

    let full: String = (0..area.height)
        .map(|y| row_text(terminal.backend().buffer(), y, area.width))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        full.contains("OpenCoder"),
        "empty session should render the tutorial containing 'OpenCoder'; got:\n{full}"
    );

    // --- State 2: first block appears → tutorial gone ---
    let mut v2 = ChatView::default();
    v2.apply(&SessionEvent::TextDelta("hello".into()));
    v2.apply(&SessionEvent::Done);
    let mut jump_btn2: Option<Rect> = None;
    let mut top_btn2: Option<Rect> = None;
    let mut body_out2: Option<Rect> = None;
    let mut scroll2 = 0u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v2,
                &Line::raw("test"),
                &mut scroll2,
                false,
                0,
                0,
                &mut body_out2,
                &mut jump_btn2,
                &mut top_btn2,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut None,
                true,
                0,
                false,
            );
        })
        .unwrap();

    let full2: String = (0..area.height)
        .map(|y| row_text(terminal.backend().buffer(), y, area.width))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !full2.contains("OpenCoder"),
        "non-empty session should NOT render the tutorial; got:\n{full2}"
    );
}

// ----- Tutorial: suppressed when viewing an empty child subagent view -----

/// An empty *child* subagent view (non-top-level) must NOT render the tutorial
/// -- only the top-level session shows it. Covers the `is_top_level &&` guard
/// added to `render_body`.
#[test]
fn empty_child_view_does_not_show_tutorial() {
    let backend = TestBackend::new(60, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let area = terminal.backend().buffer().area;

    // An empty ChatView rendered as a non-top-level (child) view.
    let v = ChatView::default();
    let mut scroll: u32 = 0;
    let mut body_out = None;
    let mut jump_btn = None;
    let mut top_btn = None;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                &Line::raw("test"),
                &mut scroll,
                false,
                0,
                0,
                &mut body_out,
                &mut jump_btn,
                &mut top_btn,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut None,
                false,
                0,
                false,
            );
        })
        .unwrap();

    let full: String = (0..area.height)
        .map(|y| row_text(terminal.backend().buffer(), y, area.width))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !full.contains("OpenCoder"),
        "empty child view should NOT render the tutorial; got:\n{full}"
    );
}

/// The body block's top border row carries the full top-title composition.
#[test]
fn body_title_row_shows_full_top_composition() {
    let v = ChatView::default(); // empty session -> tutorial path, title still rendered
    let backend = TestBackend::new(100, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut body_out: Option<Rect> = None;
    let mut jump_btn: Option<Rect> = None;
    let mut top_btn: Option<Rect> = None;
    let mut scroll = 0u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                &Line::from(vec![
                    Span::raw("/root/opencoder"),
                    Span::raw(" \u{00b7} "),
                    Span::raw("glm-5.2"),
                    Span::raw(" \u{00b7} high"),
                ]),
                &mut scroll,
                false,
                0,
                0,
                &mut body_out,
                &mut jump_btn,
                &mut top_btn,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut None,
                true,
                0,
                false,
            );
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 100);
    assert!(
        row.contains("/root/opencoder \u{00b7} glm-5.2 \u{00b7} high"),
        "body title row must show workdir · model · effort; got: {row}"
    );
}

/// When `submitted` is true (user has interacted), the in-body tutorial
/// must NOT render even if the transcript is empty — e.g. after submitting a
/// bare control command like `/plan` that adds no transcript block.
#[test]
fn submitted_hides_tutorial_even_with_empty_blocks() {
    let backend = TestBackend::new(60, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let area = terminal.backend().buffer().area;

    let v = ChatView {
        submitted: true,
        ..Default::default()
    };

    let mut scroll = 0u32;
    let mut body_out = None;
    let mut jump_btn = None;
    let mut top_btn = None;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                &Line::raw("test"),
                &mut scroll,
                false,
                0,
                0,
                &mut body_out,
                &mut jump_btn,
                &mut top_btn,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut None,
                true,
                0,
                false,
            );
        })
        .unwrap();

    let full: String = (0..area.height)
        .map(|y| row_text(terminal.backend().buffer(), y, area.width))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !full.contains("OpenCoder"),
        "tutorial must NOT render when submitted=true, even with empty blocks; got:\n{full}"
    );
}
