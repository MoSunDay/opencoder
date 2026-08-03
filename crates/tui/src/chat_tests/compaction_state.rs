use super::super::*;

/// A CompactionDelta creates a Compaction block that starts collapsed and
/// hides its content.
#[test]
fn compaction_delta_creates_collapsed_block() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::CompactionDelta("line1\nline2\nline3".into()));
    // Collapsed by default: header only, content hidden.
    let text = block_text(&v);
    assert!(text.contains("Compaction"));
    assert!(!text.contains("line1"));
    assert!(v.last_compaction_collapsed());
}

/// Toggling expands to reveal the content, then collapses again.
#[test]
fn toggle_expands_and_collapses() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::CompactionDelta("summary-a\nsummary-b".into()));
    assert!(!block_text(&v).contains("summary-a"));

    v.toggle_compaction_at(0);
    assert!(block_text(&v).contains("summary-a"));
    assert!(block_text(&v).contains("summary-b"));
    assert!(!v.last_compaction_collapsed());

    v.toggle_compaction_at(0);
    assert!(!block_text(&v).contains("summary-a"));
    assert!(v.last_compaction_collapsed());
}

/// collapse_all_collapsible collapses compaction blocks alongside thinking
/// and tool blocks.
#[test]
fn collapse_all_covers_compaction() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    v.apply(&SessionEvent::TextDelta("answer".into()));
    v.apply(&SessionEvent::Done);
    v.apply(&SessionEvent::CompactionDelta("compact".into()));

    // Expand everything so they are observably NOT collapsed beforehand.
    v.toggle_thinking_at(0);
    let compaction_idx = v.compaction_headers()[0].block_idx;
    v.toggle_compaction_at(compaction_idx);
    assert!(!v.last_compaction_collapsed());

    // Collapse all in one call.
    v.collapse_all_collapsible();

    for b in &v.blocks {
        match b {
            ChatBlock::Thinking { collapsed, .. }
            | ChatBlock::Compaction { collapsed, .. } => {
                assert!(*collapsed, "collapsible block must be collapsed");
            }
            _ => {}
        }
    }
    assert!(v.last_compaction_collapsed());
}

/// CompactionDelta appends to the current trailing Compaction block instead
/// of creating a new one.
#[test]
fn multiple_deltas_accumulate_in_one_block() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::CompactionDelta("part1\n".into()));
    v.apply(&SessionEvent::CompactionDelta("part2".into()));

    // Exactly one Compaction block with accumulated text.
    let compaction_count = v
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Compaction { .. }))
        .count();
    assert_eq!(compaction_count, 1);

    v.toggle_compaction_at(0);
    let text = block_text(&v);
    assert!(text.contains("part1"));
    assert!(text.contains("part2"));
}

/// Header shows only icon + label — no line count, no expand/collapse hint.
#[test]
fn header_text_is_clean() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::CompactionDelta("a\nb\nc".into()));

    // Collapsed: clean header.
    let flat = v.flatten();
    let header: String = flat[0].spans.iter().map(|s| &*s.content).collect();
    assert!(header.contains("Compaction"));
    assert!(!header.contains("3 lines"));
    assert!(!header.contains("expand"));

    // Expanded: still clean header (no "collapse" hint).
    v.toggle_compaction_at(0);
    let flat = v.flatten();
    let header: String = flat[0].spans.iter().map(|s| &*s.content).collect();
    assert!(header.contains("Compaction"));
    assert!(!header.contains("collapse"));
}
