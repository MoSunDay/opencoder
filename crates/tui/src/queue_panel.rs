//! Queue panel interaction logic — pure helpers for the mouse-driven
//! reorder/delete of pending follow-up (queue) items. Split out of `app.rs`
//! to keep that file under the line budget and to make the logic unit-testable.

use ratatui::layout::Rect;

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
    let i = match items.iter().position(|(s, _)| *s == seq) {
        Some(i) => i,
        None => return QueueEffect::None,
    };
    match action {
        QueueBtnAction::Delete => QueueEffect::Delete(seq),
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
        assert_eq!(plan(&items(), 20, QueueBtnAction::Submit), QueueEffect::None);
    }
}
