//! Queue panel interaction logic — pure helpers for the mouse-driven
//! reorder/delete of pending follow-up (queue) items. Split out of `app.rs`
//! to keep that file under the line budget and to make the logic unit-testable.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::composer;

/// Which control glyph a queue row's click landed on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum QueueBtnAction {
    Up,
    Down,
    Delete,
    Submit,
}

/// One mouse hit-target exported by the renderer for the event loop to test
/// clicks against. Recomputed every frame alongside `MouseHits`.
#[derive(Clone, Copy)]
pub(crate) struct QueueBtn {
    pub seq: i64,
    pub action: QueueBtnAction,
    pub rect: Rect,
}

/// What the event loop should do in response to a queue-button click.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum QueueEffect {
    None,
    Delete(i64),
    Swap(i64, i64),
}

/// Decide the effect of clicking `action` on the row carrying `seq`, given the
/// current ordered pending list (admitted_seq ASC = drain order). Pure: does not
/// mutate; returns the seq pair to swap, the seq to delete, or `None`.
pub(crate) fn plan(items: &[(i64, String)], seq: i64, action: QueueBtnAction) -> QueueEffect {
    // Delete only needs the seq — not the list index — so handle it first.
    // This lets the ✕ work for steer rows, whose seq lives in a separate
    // `steer_items` vec and is never present in `items` here.
    if action == QueueBtnAction::Delete {
        return QueueEffect::Delete(seq);
    }
    let i = match items.iter().position(|(s, _)| *s == seq) {
        Some(i) => i,
        None => return QueueEffect::None,
    };
    match action {
        QueueBtnAction::Delete => unreachable!("handled above"),
        QueueBtnAction::Up => {
            if i == 0 {
                QueueEffect::None
            } else {
                QueueEffect::Swap(seq, items[i - 1].0)
            }
        }
        QueueBtnAction::Down => {
            if i + 1 >= items.len() {
                QueueEffect::None
            } else {
                QueueEffect::Swap(seq, items[i + 1].0)
            }
        }
        // Submit is handled directly in `handle_mouse` (it returns
        // `MouseOutcome::SteerSubmit` without consulting `plan`), but the
        // arm must exist for exhaustive matching.
        QueueBtnAction::Submit => QueueEffect::None,
    }
}

/// Swap the two identified items in the local mirror after the store has
/// confirmed the reorder. Pure over the vec; no-op if either seq is absent.
pub(crate) fn apply_swap(items: &mut [(i64, String)], a: i64, b: i64) {
    let ia = items.iter().position(|(s, _)| *s == a);
    let ib = items.iter().position(|(s, _)| *s == b);
    if let (Some(ia), Some(ib)) = (ia, ib) {
        items.swap(ia, ib);
    }
}

/// X-column offsets (relative to the panel's left edge) of the three queue-row
/// control glyphs for a panel of the given width. The 6-wide trailing strip
/// `" \u{25b2} \u{25bc} \u{2715}"` occupies the last 6 columns, so the glyphs
/// sit at `width-5` (up), `width-3` (down), `width-1` (delete). The renderer
/// pads the row's head to `width-6` so the strip lands exactly here, keeping
/// the visible glyph position aligned with the hit rect. Callers must only
/// invoke this for clickable rows (`width > 10`), so the subtraction is safe.
pub(crate) fn btn_x_offsets(width: u16) -> [u16; 3] {
    [width - 5, width - 3, width - 1]
}

/// X-column offsets (relative to the panel's left edge) of the two steer-row
/// control glyphs for a panel of the given width. The 4-wide trailing strip
/// `" \u{2715} >"` occupies the last 4 columns, so delete sits at `width-3`
/// and submit at `width-1`. Mirrors `btn_x_offsets` but for the shorter
/// steer-row control strip. Callers must only invoke this for clickable rows
/// (`width > 8`), so the subtraction is safe.
pub(crate) fn steer_btn_x_offsets(width: u16) -> [u16; 2] {
    [width - 3, width - 1]
}

pub(crate) fn render_queue_panel(
    f: &mut Frame,
    area: Rect,
    steer_items: &[(i64, String)],
    queue_items: &[(i64, String)],
    btns: &mut Vec<QueueBtn>,
) {
    struct E<'a> {
        prefix: &'a str,
        text: &'a str,
        color: Color,
        seq: Option<i64>,
        is_steer: bool,
    }
    let mut entries: Vec<E> = Vec::new();
    for (seq, s) in steer_items {
        entries.push(E {
            prefix: "\u{21b3} steer",
            text: s.as_str(),
            color: Color::Blue,
            seq: Some(*seq),
            is_steer: true,
        });
    }
    for (seq, q) in queue_items {
        entries.push(E {
            prefix: "[queued]",
            text: q.as_str(),
            color: Color::Yellow,
            seq: Some(*seq),
            is_steer: false,
        });
    }
    let total = entries.len();
    if total == 0 || area.height == 0 {
        return;
    }

    let max_lines = (area.height as usize).min(3);
    let avail_w = area.width as usize;
    let overflow = total > max_lines;
    let item_capacity = if overflow {
        max_lines.saturating_sub(1)
    } else {
        max_lines
    };
    let start = total.saturating_sub(item_capacity);
    let visible = &entries[start..];

    let mut lines: Vec<Line> = Vec::new();
    if overflow {
        lines.push(Line::from(Span::styled(
            format!(" \u{2191}{} more ", start),
            Style::default().fg(Color::DarkGray),
        )));
    }
    // Clickable rows reserve a trailing control strip. Queue rows use a
    // 6-column strip (" \u{25b2} \u{25bc} \u{2715}": up/down/delete); steer
    // rows use a 4-column strip (" \u{2715} >": delete/submit). Very narrow
    // terminals render without controls. Each control glyph gets a 1-cell
    // hit rect.
    for e in visible {
        let btn_w = if e.is_steer { 4usize } else { 6usize };
        let clickable = e.seq.is_some() && avail_w > btn_w + 4;
        let cap = if clickable {
            avail_w.saturating_sub(btn_w)
        } else {
            avail_w
        };
        let head = format!(" {}: {}", e.prefix, e.text);
        let head_display = composer::truncate_to_width(&head, cap);
        let head_len = composer::str_width(&head_display);
        let mut spans: Vec<Span> = vec![Span::styled(head_display, Style::default().fg(e.color))];
        if clickable {
            let seq = e.seq.unwrap();
            let y = area.y + lines.len() as u16;
            // Right-align the control strip: pad the head out to `cap` so the
            // glyphs land at the right edge and stay aligned with the hit rects.
            let pad = cap.saturating_sub(head_len);
            if pad > 0 {
                spans.push(Span::raw(" ".repeat(pad)));
            }
            if e.is_steer {
                // Steer row: " ✕ >" — delete + submit-now.
                spans.push(Span::styled(
                    " \u{2715} >".to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
                let [del_x, sub_x] = steer_btn_x_offsets(area.width);
                btns.push(QueueBtn {
                    seq,
                    action: QueueBtnAction::Delete,
                    rect: Rect::new(area.x + del_x, y, 1, 1),
                });
                btns.push(QueueBtn {
                    seq,
                    action: QueueBtnAction::Submit,
                    rect: Rect::new(area.x + sub_x, y, 1, 1),
                });
            } else {
                // Queue row: " ▲ ▼ ✕" — up/down/delete.
                spans.push(Span::styled(
                    " \u{25b2} \u{25bc} \u{2715}".to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
                let [up_x, down_x, del_x] = btn_x_offsets(area.width);
                btns.push(QueueBtn {
                    seq,
                    action: QueueBtnAction::Up,
                    rect: Rect::new(area.x + up_x, y, 1, 1),
                });
                btns.push(QueueBtn {
                    seq,
                    action: QueueBtnAction::Down,
                    rect: Rect::new(area.x + down_x, y, 1, 1),
                });
                btns.push(QueueBtn {
                    seq,
                    action: QueueBtnAction::Delete,
                    rect: Rect::new(area.x + del_x, y, 1, 1),
                });
            }
        }
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<(i64, String)> {
        vec![(10, "a".into()), (20, "b".into()), (30, "c".into())]
    }

    #[test]
    fn up_swaps_with_predecessor() {
        assert_eq!(
            plan(&items(), 20, QueueBtnAction::Up),
            QueueEffect::Swap(20, 10)
        );
    }

    #[test]
    fn up_at_top_is_noop() {
        assert_eq!(plan(&items(), 10, QueueBtnAction::Up), QueueEffect::None);
    }

    #[test]
    fn down_swaps_with_successor() {
        assert_eq!(
            plan(&items(), 20, QueueBtnAction::Down),
            QueueEffect::Swap(20, 30)
        );
    }

    #[test]
    fn down_at_bottom_is_noop() {
        assert_eq!(plan(&items(), 30, QueueBtnAction::Down), QueueEffect::None);
    }

    #[test]
    fn delete_returns_seq() {
        assert_eq!(
            plan(&items(), 20, QueueBtnAction::Delete),
            QueueEffect::Delete(20)
        );
    }

    #[test]
    fn delete_steer_seq_absent_from_items_still_deletes() {
        // Regression: a steer row's seq lives in `steer_items`, never in
        // `queue_items`. Previously `plan` did the position lookup before
        // examining the action, so this returned `None` and the ✕ was a
        // silent no-op. Delete only needs the seq — not the index.
        assert_eq!(
            plan(&items(), 777, QueueBtnAction::Delete),
            QueueEffect::Delete(777)
        );
    }

    #[test]
    fn unknown_seq_is_noop() {
        assert_eq!(plan(&items(), 999, QueueBtnAction::Up), QueueEffect::None);
    }

    #[test]
    fn apply_swap_reorders_locally() {
        let mut it = items();
        apply_swap(&mut it, 10, 30);
        let seqs: Vec<i64> = it.iter().map(|(s, _)| *s).collect();
        assert_eq!(seqs, vec![30, 20, 10]);
    }

    #[test]
    fn apply_swap_missing_is_noop() {
        let mut it = items();
        apply_swap(&mut it, 10, 999);
        let seqs: Vec<i64> = it.iter().map(|(s, _)| *s).collect();
        assert_eq!(seqs, vec![10, 20, 30]);
    }

    #[test]
    fn btn_x_offsets_pin_glyphs_to_right_edge() {
        // Glyphs occupy the last 6 cols: " ▲ ▼ ✕" → up/down/del at -5/-3/-1.
        assert_eq!(btn_x_offsets(80), [75, 77, 79]);
        assert_eq!(btn_x_offsets(40), [35, 37, 39]);
        // Minimum clickable width is 11 (avail_w > btn_w + 4).
        assert_eq!(btn_x_offsets(11), [6, 8, 10]);
    }

    #[test]
    fn steer_btn_x_offsets_pin_glyphs_to_right_edge() {
        // Glyphs occupy the last 4 cols: " ✕ >" → del at -3, submit at -1.
        assert_eq!(steer_btn_x_offsets(80), [77, 79]);
        assert_eq!(steer_btn_x_offsets(40), [37, 39]);
        assert_eq!(steer_btn_x_offsets(11), [8, 10]);
    }

    #[test]
    fn submit_action_is_noop_in_plan() {
        assert_eq!(
            plan(&items(), 20, QueueBtnAction::Submit),
            QueueEffect::None
        );
    }
}
