//! Terminal resize helpers: detect size changes and sync ratatui's layout.
//! Extracted from `app_helpers.rs` to keep that file under the 800-line cap.

use crate::render::Term;

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

/// Handle a crossterm `Resize` event. The input pump arm already flagged the
/// frame dirty, so here we just tell ratatui the size changed so its diff
/// buffer matches the new layout (prevents glitches and keeps the persisted
/// hit-rects valid after resize).
pub(crate) fn on_resize_event(terminal: &mut Term, last_size: &mut Option<(u16, u16)>) {
    let _ = terminal.autoresize();
    // Keep last_size in sync so poll_idle_resize doesn't fire a redundant
    // autoresize + spurious re-render on the very next frame tick.
    if let Ok(rect) = terminal.size() {
        let dims = (rect.width, rect.height);
        if dims.0 > 0 && dims.1 > 0 {
            *last_size = Some(dims);
        }
    }
}

/// Idle-resize safety net: poll the kernel for the real terminal size every
/// frame and force a ratatui autoresize + redraw when it differs from
/// `last_size` (crossterm may drop a Resize event). `terminal.size()` is a
/// single ioctl (us-level); errors are ignored via `.ok()` (e.g. stdout is not
/// a tty). Updates `last_size` and returns `true` when a resize was detected so
/// the caller can mark the frame dirty.
pub(crate) fn poll_idle_resize(terminal: &mut Term, last_size: &mut Option<(u16, u16)>) -> bool {
    let Some(cur) = terminal.size().ok() else {
        return false;
    };
    let dims = (cur.width, cur.height);
    if size_changed(*last_size, dims) {
        let _ = terminal.autoresize();
        *last_size = Some(dims);
        true
    } else {
        false
    }
}
