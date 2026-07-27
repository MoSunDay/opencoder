//! Hit-rect recording for clickable block headers, extracted from `render.rs`
//! to keep `render.rs` under the file-size limit. Each function walks the
//! cached viewport layout to map a header's flat line index to an on-screen
//! row, then records a one-cell-tall click target.

use super::*;

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

/// Populate `out` with one `ToolBtn` per Tool-block header line that is
/// currently visible inside the body viewport. Mirrors `record_thinking_hits`.
#[allow(clippy::too_many_arguments)]
pub(super) fn record_tool_hits(
    chat: &ChatView,
    cache: &ViewportCache,
    text_w: u16,
    scroll_y: usize,
    visible_h: usize,
    x: u16,
    y0: u16,
    out: &mut Vec<ToolBtn>,
) {
    let headers = chat.tool_headers();
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
            out.push(ToolBtn {
                block_idx: h.block_idx,
                rect: Rect::new(x, screen_y, text_w, 1),
            });
        }
    }
}
