//! Subagent GROUP header hit-rects: `record_subagent_group_hits` maps each
//! `> subagents` / `▸ subagents` group header line to a one-row-tall,
//! full-width click target in BOTH disclosure states, while the per-row
//! `record_subagent_hits` targets exist only while expanded — and the full
//! frame wires both through `render` / `render_body` (recomputed every frame).

use super::*;
use crate::chat::ChatView;
use opencoder_session::SessionEvent;
use ratatui::Terminal;

/// One finished subagent (a single contiguous run) followed by a marker
/// line, so the run sits at the top of the transcript.
fn subagent_view(collapsed: bool) -> ChatView {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "dig".into(),
        child_session_id: "c1".into(),
    });
    v.apply(&SessionEvent::SubagentEnd {
        id: "s1".into(),
        ok: true,
        cancelled: false,
        summary: "found".into(),
    });
    v.push_marker(Line::raw("tail"));
    v.subagents_collapsed = collapsed;
    v
}

fn group_hits(v: &ChatView, scroll_y: usize) -> Vec<SubagentGroupBtn> {
    let cache = ViewportCache::build(v, 40, 0, 0);
    let mut out = Vec::new();
    hit_records::record_subagent_group_hits(v, &cache, 40, scroll_y, 10, 1, 2, &mut out);
    out
}

fn row_hits(v: &ChatView, scroll_y: usize) -> Vec<SubagentBtn> {
    let cache = ViewportCache::build(v, 40, 0, 0);
    let mut out = Vec::new();
    hit_records::record_subagent_hits(v, &cache, 40, scroll_y, 10, 1, 2, &mut out);
    out
}

/// Expanded: the group header's target sits on flattened row 0 and the
/// subagent row's target right below it. Collapsed: the header keeps its
/// target (same screen row) and no row target exists.
#[test]
fn group_header_target_in_both_states_rows_only_while_expanded() {
    let expanded = subagent_view(false);
    let groups = group_hits(&expanded, 0);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].rect, Rect::new(1, 2, 40, 1), "y0 + row 0");
    let rows = row_hits(&expanded, 0);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].block_idx, 0);
    assert_eq!(rows[0].rect, Rect::new(1, 3, 40, 1), "y0 + row 1");
    assert!(in_rect(groups[0].rect, 1, 2) && in_rect(groups[0].rect, 39, 2));
    assert!(
        !in_rect(groups[0].rect, 5, 3),
        "adjacent row is not the header"
    );

    let collapsed = subagent_view(true);
    let groups = group_hits(&collapsed, 0);
    assert_eq!(groups.len(), 1, "the header target survives collapsing");
    assert_eq!(groups[0].rect, Rect::new(1, 2, 40, 1));
    assert!(
        row_hits(&collapsed, 0).is_empty(),
        "hidden rows carry no click target"
    );
}

/// Scrolling a line above the viewport drops its target and shifts the
/// remaining ones up with the content (the row target is at `y0 + row -
/// scroll_y`); scrolling past both leaves no target.
#[test]
fn group_header_scrolled_above_is_not_hittable() {
    let v = subagent_view(false);
    assert!(group_hits(&v, 1).is_empty(), "header scrolled out above");
    let rows = row_hits(&v, 1);
    assert_eq!(rows.len(), 1, "the row is still on screen");
    assert_eq!(rows[0].rect, Rect::new(1, 2, 40, 1), "row moved up to y0");
    assert!(group_hits(&v, 2).is_empty() && row_hits(&v, 2).is_empty());
}

/// Full frame: `render` records the group header target and the row target
/// below it; collapsed, the screen shows the `▸ subagents` header only and
/// the frame records the header target with NO row target.
#[test]
fn full_frame_records_group_header_and_row_targets_per_state() {
    let draw = |v: &ChatView, hits: &mut MouseHits| {
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        let mut scroll = 0u32;
        let mut queue_scroll = 0u32;
        let mut viewport: Option<ViewportCache> = None;
        render(
            &mut terminal,
            v,
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
            &mut scroll,
            true,
            &mut queue_scroll,
            0,
            0,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            hits,
            &mut viewport,
            false,
            false,
            &[],
            false,
            None,
            None,
            0,
            0,
            true,
            "act",
            false,
            None,
            None,
        )
        .unwrap();
        terminal.backend().buffer().clone()
    };

    let mut hits = MouseHits::default();
    let buf = draw(&subagent_view(false), &mut hits);
    let text = buffer_text(&buf);
    assert!(text.contains("> subagents"), "{text}");
    assert!(text.contains("subagent [explore]"), "{text}");
    assert_eq!(hits.subagent_group_btns.len(), 1, "one group header target");
    assert_eq!(hits.subagent_btns.len(), 1, "one subagent row target");
    let group_row = hits.subagent_group_btns[0].rect.y;
    let row_row = hits.subagent_btns[0].rect.y;
    assert!(row_of(&buf, "> subagents") == Some(group_row));
    assert_eq!(row_row, group_row + 1);

    // Re-rendering the SAME hits (as the event loop does) after a collapse
    // recomputes both lists — the stale row target must not survive.
    let buf = draw(&subagent_view(true), &mut hits);
    let text = buffer_text(&buf);
    assert!(text.contains("\u{25b8} subagents"), "{text}");
    assert!(!text.contains("subagent [explore]"), "{text}");
    assert_eq!(hits.subagent_group_btns.len(), 1);
    assert!(hits.subagent_btns.is_empty());
}

fn row_of(buf: &ratatui::buffer::Buffer, needle: &str) -> Option<u16> {
    let w = buf.area.width;
    (0..buf.area.height).find(|&y| row_text(buf, y, w).contains(needle))
}

fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y, buf.area.width))
        .collect::<Vec<_>>()
        .join("\n")
}
