//! Terminal resize helpers: detect size changes and sync ratatui's layout.
//! Extracted from `app_helpers.rs` to keep that file under the 800-line cap.

use ratatui::backend::Backend;
use ratatui::Terminal;

/// Returns `true` when there is no prior reading yet (first frame) or when
/// either dimension differs. Factored out so the idle-resize detection logic
/// is unit-testable without a live terminal.
pub(crate) fn size_changed(prev: Option<(u16, u16)>, cur: (u16, u16)) -> bool {
    // Ignore 0x0 (transient glitch on minimize/detach) — it self-corrects on
    // the next real Resize event.
    if cur.0 == 0 || cur.1 == 0 {
        return false;
    }
    prev.is_none_or(|p| p != cur)
}

/// Handle a crossterm `Resize` event. Besides syncing ratatui's buffers, clear
/// the physical terminal grid. This is required under tmux: when a pane grows
/// (notably after hiding its status bar), ratatui initializes the newly exposed
/// buffer cells as blank and therefore emits no diff for them, while tmux can
/// still hold glyphs from the old grid in those same cells.
pub(crate) fn on_resize_event<B: Backend>(
    terminal: &mut Terminal<B>,
    last_size: &mut Option<(u16, u16)>,
) -> std::io::Result<()> {
    terminal.autoresize()?;
    terminal.clear()?;
    // Keep last_size in sync so poll_idle_resize doesn't fire a redundant
    // autoresize + spurious re-render on the very next frame tick.
    let rect = terminal.size()?;
    let dims = (rect.width, rect.height);
    if dims.0 > 0 && dims.1 > 0 {
        *last_size = Some(dims);
    }
    Ok(())
}

/// Idle-resize safety net: poll the kernel for the real terminal size every
/// frame and force a ratatui autoresize + redraw when it differs from
/// `last_size` (crossterm may drop a Resize event). `terminal.size()` is a
/// single ioctl (us-level). I/O failures are returned to the app instead of
/// silently leaving a partially refreshed screen. Updates `last_size` and
/// returns `true` when a resize was detected so the caller can mark the frame
/// dirty.
pub(crate) fn poll_idle_resize<B: Backend>(
    terminal: &mut Terminal<B>,
    last_size: &mut Option<(u16, u16)>,
) -> std::io::Result<bool> {
    let cur = terminal.size()?;
    let dims = (cur.width, cur.height);
    if size_changed(*last_size, dims) {
        terminal.autoresize()?;
        terminal.clear()?;
        *last_size = Some(dims);
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::widgets::Paragraph;

    fn backend_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn terminal_with_marker() -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(20, 4)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(Paragraph::new("stale-marker"), frame.area()))
            .unwrap();
        assert!(backend_text(&terminal).contains("stale-marker"));
        terminal
    }

    #[test]
    fn resize_event_clears_the_physical_grid() {
        let mut terminal = terminal_with_marker();
        let mut last_size = Some((20, 4));

        terminal.backend_mut().resize(20, 5);
        on_resize_event(&mut terminal, &mut last_size).unwrap();

        assert_eq!(last_size, Some((20, 5)));
        assert!(!backend_text(&terminal).contains("stale-marker"));
    }

    #[test]
    fn idle_resize_clears_the_physical_grid() {
        let mut terminal = terminal_with_marker();
        let mut last_size = Some((20, 4));

        terminal.backend_mut().resize(20, 5);
        let changed = poll_idle_resize(&mut terminal, &mut last_size).unwrap();

        assert!(changed);
        assert_eq!(last_size, Some((20, 5)));
        assert!(!backend_text(&terminal).contains("stale-marker"));
    }

    #[test]
    fn idle_poll_without_resize_does_not_clear() {
        let mut terminal = terminal_with_marker();
        let mut last_size = Some((20, 4));

        let changed = poll_idle_resize(&mut terminal, &mut last_size).unwrap();

        assert!(!changed);
        assert!(backend_text(&terminal).contains("stale-marker"));
    }
}
