use super::*;

/// Build a ChatView with a Compaction block followed by an assistant block.
fn compaction_view() -> ChatView {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::CompactionDelta("summary-a\nsummary-b".into()));
    v.apply(&SessionEvent::TextDelta("answer".into()));
    v.apply(&SessionEvent::Done);
    v
}

/// A collapsed compaction header at the top is visible at scroll 0 and gets
/// a full-width hit rect on its header row.
#[test]
fn collapsed_header_visible_gets_hit_rect() {
    let v = compaction_view();
    let cache = crate::render_viewport::ViewportCache::build(&v, 40, 0);
    let headers = v.compaction_headers();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].header_line_idx, 0);

    let mut hits = Vec::new();
    super::hit_records::record_compaction_hits(&v, &cache, 40, 0, 10, 1, 2, &mut hits);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].block_idx, headers[0].block_idx);
    assert_eq!(hits[0].rect, Rect::new(1, 2, 40, 1));
}

/// Expanding the compaction block grows its rendered lines but the header
/// stays at the same screen row.
#[test]
fn expanded_header_row_unchanged() {
    let mut v = compaction_view();
    v.toggle_compaction_at(v.compaction_headers()[0].block_idx);
    let lines = v.flatten();
    let cache = crate::render_viewport::ViewportCache::build(&v, 40, 0);
    let mut hits = Vec::new();
    super::hit_records::record_compaction_hits(&v, &cache, 40, 0, 10, 1, 2, &mut hits);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].rect, Rect::new(1, 2, 40, 1));
    // Content lines are now present in the flattened output.
    assert!(lines
        .iter()
        .any(|l| { l.spans.iter().any(|s| s.content.contains("summary-a")) }));
}

/// Scrolling past the header removes its hit rect.
#[test]
fn header_scrolled_above_is_not_hittable() {
    let v = compaction_view();
    let cache = crate::render_viewport::ViewportCache::build(&v, 40, 0);
    let mut hits = Vec::new();
    super::hit_records::record_compaction_hits(&v, &cache, 40, 1, 10, 1, 2, &mut hits);
    assert!(
        hits.is_empty(),
        "header above viewport should not be hittable"
    );
}

/// No compaction blocks means no hits.
#[test]
fn no_compaction_blocks_means_no_hits() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("just text".into()));
    v.apply(&SessionEvent::Done);
    let cache = crate::render_viewport::ViewportCache::build(&v, 40, 0);
    let mut hits = Vec::new();
    super::hit_records::record_compaction_hits(&v, &cache, 40, 0, 10, 1, 2, &mut hits);
    assert!(hits.is_empty());
}

/// in_rect matches a click on the header row and misses other rows.
#[test]
fn hit_rect_matches_click_on_header_row() {
    let v = compaction_view();
    let cache = crate::render_viewport::ViewportCache::build(&v, 40, 0);
    let mut hits = Vec::new();
    super::hit_records::record_compaction_hits(&v, &cache, 40, 0, 10, 1, 2, &mut hits);
    let rect = hits[0].rect;
    assert!(in_rect(rect, 5, 2));
    assert!(in_rect(rect, 1, 2));
    assert!(!in_rect(rect, 5, 1));
    assert!(!in_rect(rect, 5, 3));
}

/// Collapsed header shows icon + label and the line count, no expand hint.
#[test]
fn collapsed_header_shows_line_count() {
    let v = compaction_view();
    let flat = v.flatten();
    let header = &flat[0];
    let text: String = header.spans.iter().map(|s| &*s.content).collect();
    assert!(text.contains("Compaction"), "header must contain label");
    // compaction_view() body is "summary-a\nsummary-b" -> 2 lines.
    assert!(text.contains("2 lines"), "collapsed header shows line count");
    assert!(!text.contains("expand"), "no expand hint");
}
