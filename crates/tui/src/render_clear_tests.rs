//! Regression tests for stale terminal cells across redraw and process
//! boundaries. Ratatui must blank cells vacated by a shorter frame; application
//! startup additionally clears the real terminal grid left by an older run.

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
    keymap_menu: Option<&crate::keymap_menu::KeymapMenu>,
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
        &Line::raw("title"),
        false,
        0,
        0,
        200_000,
        200_000,
        "idle",
        &[],
        &[],
        scroll,
        true,
        queue_scroll,
        0,
        0,
        None,
        None,
        None,
        None,
        None,
        None,
        keymap_menu,
        hits,
        viewport,
        None,
        None,
        &[],
        false,
        None,
        0,
        0,
        true,
        false,
        "act",
    )
    .unwrap();
}

/// Frame 1 paints a distinctive marker; frames 2 and 3 paint an empty body.
/// This pins ratatui's two-buffer lifecycle: vacated cells remain blank when
/// either buffer becomes current again. The self-check prevents a vacuous pass.
#[test]
fn shorter_frames_keep_vacated_cells_blank_across_buffer_swaps() {
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
        None,
        &mut hits,
        &mut viewport,
    );
    assert!(
        buffer_text(terminal.backend().buffer()).contains("markerword"),
        "frame 1 must paint the marker into the body (else the regression \
         assertion below is vacuous)"
    );

    // Frames 2 and 3 exercise both sides of ratatui's diff buffer.
    let chat_b = ChatView::default();
    draw_frame(
        &mut terminal,
        &chat_b,
        &mut scroll,
        &mut queue_scroll,
        None,
        &mut hits,
        &mut viewport,
    );
    draw_frame(
        &mut terminal,
        &chat_b,
        &mut scroll,
        &mut queue_scroll,
        None,
        &mut hits,
        &mut viewport,
    );

    let text = buffer_text(terminal.backend().buffer());
    assert!(
        !text.contains("markerword"),
        "vacated cells must stay blank; found leftover: {text:?}"
    );
}

/// Thinking content frequently changes length while streaming. A shorter new
/// snapshot must blank every cell occupied by the previous expanded block,
/// including when ratatui rotates back to the older side of its double buffer.
#[test]
fn shorter_thinking_frame_never_reveals_old_lines() {
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    let mut scroll = 0u32;
    let mut queue_scroll = 0u32;
    let mut hits = MouseHits::default();
    let mut viewport = None;

    let mut old = ChatView::default();
    old.apply(&SessionEvent::ReasoningDelta(
        "old-overlap-marker\nold-tail".into(),
    ));
    old.toggle_thinking_at(0);
    draw_frame(
        &mut terminal,
        &old,
        &mut scroll,
        &mut queue_scroll,
        None,
        &mut hits,
        &mut viewport,
    );
    assert!(buffer_text(terminal.backend().buffer()).contains("old-overlap-marker"));

    let mut new = ChatView::default();
    new.apply(&SessionEvent::ReasoningDelta("new".into()));
    new.toggle_thinking_at(0);
    for _ in 0..2 {
        draw_frame(
            &mut terminal,
            &new,
            &mut scroll,
            &mut queue_scroll,
            None,
            &mut hits,
            &mut viewport,
        );
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains("new"),
            "new Thinking content missing: {text:?}"
        );
        assert!(
            !text.contains("old-overlap-marker") && !text.contains("old-tail"),
            "old Thinking content leaked into the new frame: {text:?}"
        );
    }
}

/// Regression: the startup `Terminal::clear()` added to `app_bootstrap` (after
/// entering the alt screen) must wipe glyphs persisted by the *previous run*.
/// tmux keeps one alt-screen grid per pane: a fresh ratatui `Terminal` starts
/// with empty buffers, so the first draw's diff (empty vs empty) emits no
/// bytes for the trailing cells — the old frame stays visible forever unless
/// the startup path issues a real clear (ESC[2J). `TestBackend` models the
/// persistent grid: its content survives unless `Terminal::clear()` resets it.
///
/// The test paints a marker (the previous run's last frame), resets the ratatui
/// buffers to the "fresh run" state WITHOUT clearing the backend, and shows the
/// marker persisting through an empty frame — then applies the fix and shows it
/// gone. The ghost step doubles as a self-check: without it the final assertion
/// would be vacuous.
#[test]
fn startup_clear_wipes_glyphs_persisted_by_previous_run() {
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    let mut scroll = 0u32;
    let mut queue_scroll: u32 = 0;
    let mut hits = MouseHits::default();
    let mut viewport: Option<ViewportCache> = None;

    // Frame 1: body holds a distinctive marker — the previous run's last frame
    // left on the tmux alt-screen grid.
    let mut chat_a = ChatView::default();
    chat_a.apply(&SessionEvent::TextDelta("markerword\n".into()));
    chat_a.apply(&SessionEvent::Done);
    draw_frame(
        &mut terminal,
        &chat_a,
        &mut scroll,
        &mut queue_scroll,
        None,
        &mut hits,
        &mut viewport,
    );
    assert!(
        buffer_text(terminal.backend().buffer()).contains("markerword"),
        "frame 1 must paint the marker into the body (else the regression \
         assertion below is vacuous)"
    );

    // Simulate the pre-fix state at the start of a NEW run: the ratatui
    // buffers are fresh (as if a new Terminal was created — swap_buffers
    // resets the marker frame out of the buffer side) while the terminal
    // grid (TestBackend) still holds the old frame. Drawing an empty frame
    // then diffs empty vs empty — no bytes are emitted — so the stale marker
    // persists.
    terminal.swap_buffers();
    terminal.current_buffer_mut().reset();
    let chat_b = ChatView::default();
    draw_frame(
        &mut terminal,
        &chat_b,
        &mut scroll,
        &mut queue_scroll,
        None,
        &mut hits,
        &mut viewport,
    );
    assert!(
        buffer_text(terminal.backend().buffer()).contains("markerword"),
        "precondition: without a startup clear the stale glyph must persist \
         through an empty diff (this is the ghost the fix removes)"
    );

    // The fix: a real `Terminal::clear()` — ESC[2J to the grid plus a reset
    // diff baseline. The next empty frame must not resurrect the marker.
    terminal.clear().unwrap();
    draw_frame(
        &mut terminal,
        &chat_b,
        &mut scroll,
        &mut queue_scroll,
        None,
        &mut hits,
        &mut viewport,
    );
    let text = buffer_text(terminal.backend().buffer());
    assert!(
        !text.contains("markerword"),
        "startup clear must wipe glyphs persisted by the previous run; \
         leftover: {text:?}"
    );
}
