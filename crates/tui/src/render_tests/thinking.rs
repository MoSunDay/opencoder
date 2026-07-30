use super::*;

/// A collapsed thinking header at the top is visible at scroll 0 and gets
/// a full-width hit rect on its header row.
#[test]
fn collapsed_header_visible_gets_hit_rect() {
    let v = thinking_view();
    let cache = crate::render_viewport::ViewportCache::build(&v, 40, 0);
    // Header is the first line (line index 0).
    let headers = v.thinking_headers();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].header_line_idx, 0);

    let mut hits = Vec::new();
    super::hit_records::record_thinking_hits(&v, &cache, 40, 0, 10, 1, 2, &mut hits);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].block_idx, headers[0].block_idx);
    // screen_y = y0 + (0 - 0) = 2; full text width.
    assert_eq!(hits[0].rect, Rect::new(1, 2, 40, 1));
}

/// Expanding the thinking block grows its rendered lines but the header
/// stays at the same screen row (row 0 → screen y0).
#[test]
fn expanded_header_row_unchanged() {
    let mut v = thinking_view();
    v.toggle_thinking_at(v.thinking_headers()[0].block_idx);
    let lines = v.flatten();
    let cache = crate::render_viewport::ViewportCache::build(&v, 40, 0);
    let mut hits = Vec::new();
    super::hit_records::record_thinking_hits(&v, &cache, 40, 0, 10, 1, 2, &mut hits);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].rect, Rect::new(1, 2, 40, 1));
    // Content lines are now present in the flattened output.
    assert!(lines
        .iter()
        .any(|l| { l.spans.iter().any(|s| s.content.contains("think-a-1")) }));
}

/// Scrolling past the header removes its hit rect (header scrolled out of
/// view above).
#[test]
fn header_scrolled_above_is_not_hittable() {
    let v = thinking_view();
    let cache = crate::render_viewport::ViewportCache::build(&v, 40, 0);
    let mut hits = Vec::new();
    // scroll_y = 1 pushes the row-0 header above the viewport.
    super::hit_records::record_thinking_hits(&v, &cache, 40, 1, 10, 1, 2, &mut hits);
    assert!(
        hits.is_empty(),
        "header above viewport should not be hittable"
    );
}

/// No thinking blocks ⇒ no work and no hits.
#[test]
fn no_thinking_blocks_means_no_hits() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("just text".into()));
    v.apply(&SessionEvent::Done);
    let cache = crate::render_viewport::ViewportCache::build(&v, 40, 0);
    let mut hits = Vec::new();
    super::hit_records::record_thinking_hits(&v, &cache, 40, 0, 10, 1, 2, &mut hits);
    assert!(hits.is_empty());
}

/// in_rect matches a click on the header row and misses other rows.
#[test]
fn hit_rect_matches_click_on_header_row() {
    let v = thinking_view();
    let cache = crate::render_viewport::ViewportCache::build(&v, 40, 0);
    let mut hits = Vec::new();
    super::hit_records::record_thinking_hits(&v, &cache, 40, 0, 10, 1, 2, &mut hits);
    let rect = hits[0].rect;
    // Click anywhere on the header row (y == 2) within x..x+width hits.
    assert!(in_rect(rect, 5, 2));
    assert!(in_rect(rect, 1, 2));
    // Adjacent rows do not hit.
    assert!(!in_rect(rect, 5, 1));
    assert!(!in_rect(rect, 5, 3));
}
