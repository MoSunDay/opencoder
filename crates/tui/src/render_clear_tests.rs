//! Tests for the per-frame `Clear` added to `render` — see the comment at the
//! top of `render`'s `terminal.draw` closure. Without that full-area clear,
//! ratatui's double-buffering reuses a buffer that still holds the previous
//! frame's glyphs; on the third draw (when that buffer becomes current again)
//! the stale content is diffed back onto the screen as "remnants around the
//! edges".

use super::*;
use crate::chat::ChatView;
use opencoder_session::SessionEvent;
use ratatui::backend::TestBackend;

/// Concatenate every cell's symbol row-by-row into a single searchable
/// string — used to detect stale glyphs that should have been cleared.
fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area;
    let mut s = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}

/// Render one frame with a fixed set of benign arguments, so the three-frame
/// scenario below stays readable. Mirrors the render call shape used in
/// `render_then_click_arrow_targets_jump_view`.
fn draw_frame(
    terminal: &mut Terminal<TestBackend>,
    chat: &ChatView,
    scroll: &mut u32,
    queue_scroll: &mut u32,
    hits: &mut MouseHits,
    viewport: &mut Option<ViewportCache>,
) {
    // Force the body viewport to rebuild for THIS chat each frame, so the
    // only thing that can leave stale glyphs is ratatui's buffer reuse —
    // which is exactly what the per-frame Clear exists to defeat.
    *viewport = None;
    render(
        terminal,
        chat,
        "",
        0,
        "title",
        "agent",
        false,
        false,
        0,
        0,
        200_000,
        200_000,
        "model",
        "idle",
        &[],
        &[],
        scroll,
        true,
        queue_scroll,
        0,
        None,
        None,
        None,
        None,
        None,
        None,
        hits,
        viewport,
        None,
        None,
        &[],
        false,
        None,
        0,
        0u16,
        true,
        false,
    )
    .unwrap();
}

/// Regression: `render` blanks the whole frame with `Clear` before painting
/// the widgets. Frame 1 paints a distinctive marker into the body; frames 2
/// and 3 paint an empty body. On frame 3 the buffer holding frame 1's glyphs
/// becomes current again, so without the per-frame `Clear` the marker would
/// re-emerge via the diff. The self-check on frame 1 ensures the marker was
/// actually painted (otherwise the regression assertion would be vacuous).
#[test]
fn per_frame_clear_wipes_stale_glyphs_across_frames() {
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    let mut scroll = 0u32;
    let mut queue_scroll: u32 = 0;
    let mut hits = MouseHits::default();
    let mut viewport: Option<ViewportCache> = None;

    // Frame 1: body holds a distinctive marker word.
    let mut chat_a = ChatView::default();
    chat_a.apply(&SessionEvent::TextDelta("markerword\n".into()));
    chat_a.apply(&SessionEvent::Done);
    draw_frame(
        &mut terminal,
        &chat_a,
        &mut scroll,
        &mut queue_scroll,
        &mut hits,
        &mut viewport,
    );
    assert!(
        buffer_text(terminal.backend().buffer()).contains("markerword"),
        "frame 1 must paint the marker into the body (else the regression \
         assertion below is vacuous)"
    );

    // Frames 2 and 3: empty body. On frame 3 the buffer holding frame 1's
    // glyphs becomes current again; only the per-frame Clear prevents them
    // from re-emerging via the diff.
    let chat_b = ChatView::default();
    draw_frame(
        &mut terminal,
        &chat_b,
        &mut scroll,
        &mut queue_scroll,
        &mut hits,
        &mut viewport,
    );
    draw_frame(
        &mut terminal,
        &chat_b,
        &mut scroll,
        &mut queue_scroll,
        &mut hits,
        &mut viewport,
    );

    let text = buffer_text(terminal.backend().buffer());
    assert!(
        !text.contains("markerword"),
        "per-frame Clear must wipe stale glyphs; found leftover: {text:?}"
    );
}
