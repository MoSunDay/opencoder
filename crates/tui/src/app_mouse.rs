//! Mouse-event handling extracted from `app_helpers.rs` to keep that file
//! under the 800-line iteration cap. Everything here is re-exported by
//! `app_helpers`, so existing `crate::app_helpers::*` call sites and tests
//! keep resolving unchanged.

use std::path::Path;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use opencoder_store::Store;

use super::sys_tokens_for;
use crate::chat::ChatView;
use crate::queue_panel;
use crate::render::{in_rect, MouseHits};

/// Outcome of a mouse event: `None` for normal handling (all effects are side
/// effects on the caller's locals), or `SteerSubmit` when the user clicked the
/// `>` submit-now button on a steer row, signalling the caller to interrupt the
/// current turn and restart the drain loop to promote pending steers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MouseOutcome {
    None,
    SteerSubmit,
}

/// Which `ChatView` a header click toggles: the focused subagent's child view
/// when one is active, else the parent. `None` (click still consumed) for a
/// stale or non-Subagent focus index.
pub(crate) fn collapse_view(chat: &mut ChatView, focus: Option<usize>) -> Option<&mut ChatView> {
    let i = match focus {
        None => return Some(chat),
        Some(i) => i,
    };
    match chat.blocks.get_mut(i)? {
        crate::chat::ChatBlock::Subagent { view, .. } => Some(view),
        _ => None,
    }
}

/// Mouse-event handler extracted from `app.rs`'s main event loop. Owns all the
/// state it touches via mutable references, so most effects are side effects on
/// the caller's locals; the exception is `SteerSubmit` which the caller must
/// handle by restarting the drain loop. `async` because the queue-panel
/// delete/swap paths call through the `Store` trait (`delete_input` /
/// `swap_input_order`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_mouse(
    m: MouseEvent,
    hits: &MouseHits,
    scroll: &mut u32,
    follow: &mut bool,
    chat: &mut ChatView,
    subagent_focus: &mut Option<usize>,
    subagent_sys: &mut u64,
    workdir: &Path,
    queue_items: &mut Vec<(i64, String)>,
    session_id: &str,
    store: &dyn Store,
    queue_scroll: &mut u32,
    pending_images: &mut Vec<(String, String)>,
) -> MouseOutcome {
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Follow button: highest-priority check so a quick succession of
            // body-click + arrow-click does not have the arrow-click swallowed.
            if let Some(r) = hits.jump_btn {
                if in_rect(r, m.column, m.row) {
                    *follow = true;
                    return MouseOutcome::None; // deterministic jump to bottom
                }
            }

            // Top-jump button: scroll back to the very first row. Sits next to
            // the jump_btn check.
            if let Some(r) = hits.top_btn {
                if in_rect(r, m.column, m.row) {
                    *scroll = 0;
                    *follow = false;
                    return MouseOutcome::None; // jump to top
                }
            }

            // ── Button-hit detection ──
            // Queue / Thinking / Subagent affordances must respond on the
            // FIRST click (no early-return guard before the toggle loop).
            let mut consumed = false;
            for btn in &hits.queue_btns {
                if !in_rect(btn.rect, m.column, m.row) {
                    continue;
                }
                consumed = true;
                // Submit-now on a steer row: signal the caller to interrupt
                // and restart the drain loop. No store mutation needed — the
                // steers are promoted by `claim_steers()` at the top of the
                // next `run_loop` iteration.
                if btn.action == queue_panel::QueueBtnAction::Submit {
                    return MouseOutcome::SteerSubmit;
                }
                match queue_panel::plan(queue_items, btn.seq, btn.action) {
                    queue_panel::QueueEffect::Delete(seq) => {
                        if store.delete_input(seq).await.is_ok() {
                            queue_items.retain(|(s, _)| *s != seq);
                            // Retain the focused view's mirror too: while a
                            // live subagent is focused the panel renders the
                            // child's `steer_items`, so removing only the
                            // parent mirror would leave the clicked row on
                            // screen as a ghost.
                            if let Some(view) = collapse_view(chat, *subagent_focus) {
                                view.steer_items.retain(|(s, _)| *s != seq);
                            }
                            chat.steer_items.retain(|(s, _)| *s != seq);
                        }
                    }
                    queue_panel::QueueEffect::Swap(a, b) => {
                        if store.swap_input_order(session_id, a, b).await.is_ok() {
                            queue_panel::apply_swap(queue_items, a, b);
                        }
                    }
                    queue_panel::QueueEffect::None => {}
                }
                break;
            }
            // Attachment ✕ delete: remove the clicked pending image on the
            // FIRST click, like the queue buttons. Rects are rebuilt every
            // frame and a single click removes one image, so the index stays
            // valid for this event only.
            for btn in &hits.attach_del_btns {
                if in_rect(btn.rect, m.column, m.row) {
                    if btn.index < pending_images.len() {
                        pending_images.remove(btn.index);
                    }
                    consumed = true;
                    break;
                }
            }
            // Click a Thinking/Tool header to toggle its collapse (subagent-aware:
            // toggles the focused child view, not the parent).
            for btn in &hits.thinking_btns {
                if in_rect(btn.rect, m.column, m.row) {
                    if let Some(v) = collapse_view(chat, *subagent_focus) {
                        ChatView::toggle_thinking_at(v, btn.block_idx);
                    }
                    consumed = true;
                    break;
                }
            }
            // Click a single tool call's header row (List state): toggle only
            // that call's output. Checked BEFORE the group line so the finer
            // target wins when rects ever overlap.
            for btn in &hits.tool_call_btns {
                if in_rect(btn.rect, m.column, m.row) {
                    if let Some(v) = collapse_view(chat, *subagent_focus) {
                        ChatView::toggle_tool_call_at(v, btn.block_idx, btn.call_idx);
                    }
                    consumed = true;
                    break;
                }
            }
            for btn in &hits.tool_btns {
                if in_rect(btn.rect, m.column, m.row) {
                    if let Some(v) = collapse_view(chat, *subagent_focus) {
                        ChatView::cycle_tool_group_at(v, btn.block_idx);
                    }
                    consumed = true;
                    break;
                }
            }
            for btn in &hits.compaction_btns {
                if in_rect(btn.rect, m.column, m.row) {
                    if let Some(v) = collapse_view(chat, *subagent_focus) {
                        ChatView::toggle_compaction_at(v, btn.block_idx);
                    }
                    consumed = true;
                    break;
                }
            }
            // Click on a Subagent-block header: enter
            // the subagent's perspective (ctx-switch).
            // No inline expansion — the child view and
            // its context stats are shown full-body.
            for btn in &hits.subagent_btns {
                if in_rect(btn.rect, m.column, m.row) {
                    *scroll = 0;
                    *follow = true;
                    *subagent_focus = Some(btn.block_idx);
                    // Cache subagent's system-prompt
                    // token estimate once on entry.
                    if let Some(crate::chat::ChatBlock::Subagent { kind, .. }) =
                        chat.blocks.get(btn.block_idx)
                    {
                        *subagent_sys = sys_tokens_for(kind, workdir, None);
                    }
                    consumed = true;
                    break;
                }
            }
            if consumed {
                return MouseOutcome::None;
            }
        }
        MouseEventKind::ScrollUp => {
            // Wheel-up over the queue/steer panel looks at older entries (toward the top; rects never overlap the body).
            if let Some(r) = hits.queue_panel {
                if in_rect(r, m.column, m.row) {
                    *queue_scroll = queue_scroll.saturating_sub(1);
                    return MouseOutcome::None;
                }
            }
            if let Some(r) = hits.body {
                if in_rect(r, m.column, m.row) {
                    *scroll = scroll.saturating_sub(8);
                    *follow = false;
                }
            }
        }
        MouseEventKind::ScrollDown => {
            // Wheel-down over the queue/steer panel moves toward newer entries (toward the bottom).
            if let Some(r) = hits.queue_panel {
                if in_rect(r, m.column, m.row) {
                    // Clamp to the cached panel total (mirrors the body clamp) so burst wheels can't overshoot.
                    let max_scroll = hits.queue_total.saturating_sub(r.height as usize);
                    *queue_scroll = queue_scroll.saturating_add(1).min(max_scroll as u32);
                    return MouseOutcome::None;
                }
            }
            if let Some(r) = hits.body {
                if in_rect(r, m.column, m.row) {
                    let visible_h = r.height.saturating_sub(2) as usize;
                    // Use cached total_rows from the last render_body call instead
                    // of re-flattening the entire transcript on every wheel event.
                    let total_rows = hits.total_rows;
                    let max_rows = total_rows.saturating_sub(visible_h);
                    *scroll = scroll.saturating_add(3);
                    if (*scroll as usize) >= max_rows {
                        *follow = true;
                    }
                }
            }
        }
        _ => {}
    }
    MouseOutcome::None
}
