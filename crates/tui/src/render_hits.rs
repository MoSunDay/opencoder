//! Hit-rect recording for clickable block headers, extracted from `render.rs`
//! to keep `render.rs` under the file-size limit. Each function walks the
//! cached viewport layout to map a header's flat line index to an on-screen
//! row, then records a one-cell-tall click target.
//!
//! The `MouseHits` aggregate and the button types live here too (also
//! extracted from `render.rs`); `render.rs` re-exports them so
//! `crate::render::MouseHits` paths stay stable.

use super::*;

/// Mouse hit-targets exported by `render` for the event loop to test clicks
/// and wheel scrolls against. Recomputed every frame.
#[derive(Default)]
pub(crate) struct MouseHits {
    pub jump_btn: Option<Rect>,
    pub top_btn: Option<Rect>,
    pub body: Option<Rect>,
    /// Queue/steer panel area (Some while the panel is visible), used by the
    /// scroll-wheel handler to scroll the panel instead of the body.
    pub queue_panel: Option<Rect>,
    /// Cached total pending entries (steer + queue) from the last render.
    /// Mirrors `total_rows` for the body: lets the wheel handler clamp the
    /// queue scroll without re-deriving the panel contents.
    pub queue_total: usize,
    pub queue_btns: Vec<QueueBtn>,
    /// Clickable ✕ delete buttons on pending-image attachment badges; one
    /// per attachment row, recomputed every frame.
    pub attach_del_btns: Vec<AttachDelBtn>,
    /// Clickable Thinking-block header rows; clicking toggles collapse.
    /// One entry per Thinking block currently visible in the body viewport.
    pub thinking_btns: Vec<ThinkingBtn>,
    /// Clickable Subagent-block header rows; clicking toggles collapse.
    pub subagent_btns: Vec<SubagentBtn>,
    /// Clickable rows inside `StepGroup` blocks — turn, step, and each open
    /// step's function-call rows, in render order.
    pub tool_call_btns: Vec<ToolCallBtn>,
    /// Clickable Compaction-block header rows; clicking toggles collapse.
    pub compaction_btns: Vec<CompactionBtn>,
    pub keymap_btns: Vec<Rect>,
    /// Cached total content rows from the last render_body call. Used by
    /// the scroll-wheel handler to clamp scroll without re-flattening.
    pub total_rows: usize,
}

/// A clickable Thinking-block header. `block_idx` indexes `ChatView::blocks`;
/// `rect` is the on-screen row of the header line.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ThinkingBtn {
    pub block_idx: usize,
    pub rect: Rect,
}

/// A clickable Subagent-block header.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SubagentBtn {
    pub block_idx: usize,
    pub rect: Rect,
}

/// A clickable row inside a `StepGroup`'s ladder. `call_idx` is the FLAT
/// index into the group's visible rows (turn row, step rows, function-call
/// rows in render order — the same walk
/// `collect_headers` and `visible_targets` enumerate); clicking toggles that
/// turn / step / single call's result.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ToolCallBtn {
    pub block_idx: usize,
    pub call_idx: usize,
    pub rect: Rect,
}

/// Populate `out` with one `ThinkingBtn` per Thinking-block header line that is
/// currently visible inside the body viewport. Uses the cached viewport layout
/// for O(headers) row lookups instead of walking all flattened lines.
#[allow(clippy::too_many_arguments)]
pub(super) fn record_thinking_hits(
    chat: &ChatView,
    cache: &ViewportCache,
    text_w: u16,
    scroll_y: usize,
    visible_h: usize,
    x: u16,
    y0: u16,
    out: &mut Vec<ThinkingBtn>,
) {
    let headers = chat.thinking_headers();
    if headers.is_empty() || visible_h == 0 || text_w == 0 || cache.total_rows() == 0 {
        return;
    }
    let viewport_bottom = scroll_y + visible_h;
    for h in headers {
        let header_row = cache.row_of_line(h.header_line_idx);
        if header_row >= viewport_bottom {
            break;
        }
        if header_row >= scroll_y {
            let screen_y = y0.saturating_add((header_row - scroll_y) as u16);
            out.push(ThinkingBtn {
                block_idx: h.block_idx,
                rect: Rect::new(x, screen_y, text_w, 1),
            });
        }
    }
}

/// Populate `out` with one `SubagentBtn` per Subagent-block header line that is
/// currently visible inside the body viewport. Mirrors `record_thinking_hits`.
#[allow(clippy::too_many_arguments)]
pub(super) fn record_subagent_hits(
    chat: &ChatView,
    cache: &ViewportCache,
    text_w: u16,
    scroll_y: usize,
    visible_h: usize,
    x: u16,
    y0: u16,
    out: &mut Vec<SubagentBtn>,
) {
    let headers = chat.subagent_headers();
    if headers.is_empty() || visible_h == 0 || text_w == 0 || cache.total_rows() == 0 {
        return;
    }
    let viewport_bottom = scroll_y + visible_h;
    for h in headers {
        let header_row = cache.row_of_line(h.header_line_idx);
        if header_row >= viewport_bottom {
            break;
        }
        if header_row >= scroll_y {
            let screen_y = y0.saturating_add((header_row - scroll_y) as u16);
            out.push(SubagentBtn {
                block_idx: h.block_idx,
                rect: Rect::new(x, screen_y, text_w, 1),
            });
        }
    }
}

/// A clickable Compaction-block header. Mirrors `ThinkingBtn`; clicking
/// toggles the block's collapse state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CompactionBtn {
    pub block_idx: usize,
    pub rect: Rect,
}

/// Populate `out` with one `CompactionBtn` per Compaction-block header line
/// that is currently visible inside the body viewport. Mirrors
/// `record_thinking_hits`.
#[allow(clippy::too_many_arguments)]
pub(super) fn record_compaction_hits(
    chat: &ChatView,
    cache: &ViewportCache,
    text_w: u16,
    scroll_y: usize,
    visible_h: usize,
    x: u16,
    y0: u16,
    out: &mut Vec<CompactionBtn>,
) {
    let headers = chat.compaction_headers();
    if headers.is_empty() || visible_h == 0 || text_w == 0 || cache.total_rows() == 0 {
        return;
    }
    let viewport_bottom = scroll_y + visible_h;
    for h in headers {
        let header_row = cache.row_of_line(h.header_line_idx);
        if header_row >= viewport_bottom {
            break;
        }
        if header_row >= scroll_y {
            let screen_y = y0.saturating_add((header_row - scroll_y) as u16);
            out.push(CompactionBtn {
                block_idx: h.block_idx,
                rect: Rect::new(x, screen_y, text_w, 1),
            });
        }
    }
}

/// Populate `out` with one `ToolCallBtn` per clickable ladder row that is
/// currently visible inside the body viewport: turn rows, step rows (while
/// their turn is open), and function-call rows (while their step is open).
#[allow(clippy::too_many_arguments)]
pub(super) fn record_tool_call_hits(
    chat: &ChatView,
    cache: &ViewportCache,
    text_w: u16,
    scroll_y: usize,
    visible_h: usize,
    x: u16,
    y0: u16,
    out: &mut Vec<ToolCallBtn>,
) {
    let headers = chat.tool_call_headers();
    if headers.is_empty() || visible_h == 0 || text_w == 0 || cache.total_rows() == 0 {
        return;
    }
    let viewport_bottom = scroll_y + visible_h;
    for h in headers {
        let header_row = cache.row_of_line(h.header_line_idx);
        if header_row >= viewport_bottom {
            break;
        }
        if header_row >= scroll_y {
            let screen_y = y0.saturating_add((header_row - scroll_y) as u16);
            out.push(ToolCallBtn {
                block_idx: h.block_idx,
                call_idx: h.call_idx,
                rect: Rect::new(x, screen_y, text_w, 1),
            });
        }
    }
}
