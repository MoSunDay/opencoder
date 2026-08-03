use super::super::*;

/// CompactionDelta events are ignored: they must not create any block or render
/// any text. Only the final `Compaction(summary)` event renders the block,
/// avoiding a flicker where `TranscriptReset` destroys the streamed block and
/// `Compaction` recreates it. Mirrors the headless CLI (display.rs).
#[test]
fn compaction_delta_is_ignored() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::CompactionDelta("streamed-chunk".into()));

    // No block is created by the delta.
    assert!(v.blocks.is_empty(), "CompactionDelta must not create a block");

    // The delta text is not rendered anywhere.
    assert!(!block_text(&v).contains("streamed-chunk"));

    // Multiple deltas still accumulate nothing.
    v.apply(&SessionEvent::CompactionDelta("more-text".into()));
    assert!(v.blocks.is_empty());
    assert!(!block_text(&v).contains("more-text"));

    // The final Compaction event still renders exactly one block, proving the
    // deltas left no half-built block behind.
    v.apply(&SessionEvent::Compaction("final-summary".into()));
    assert_eq!(v.blocks.len(), 1, "Compaction must create exactly one block");
    assert!(v.last_compaction_collapsed());
}

/// A Compaction event creates a Compaction block that starts collapsed and
/// hides its content.
#[test]
fn compaction_creates_collapsed_block() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::Compaction("line1\nline2\nline3".into()));
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
    v.apply(&SessionEvent::Compaction("summary-a\nsummary-b".into()));
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
    v.apply(&SessionEvent::Compaction("compact".into()));

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

/// Collapsed header shows icon + label + line count; expanded header does not.
#[test]
fn header_text_shows_line_count() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::Compaction("a\nb\nc".into()));

    // Collapsed: body "a\nb\nc" -> 3 lines, shown in the header.
    let flat = v.flatten();
    let header: String = flat[0].spans.iter().map(|s| &*s.content).collect();
    assert!(header.contains("Compaction"));
    assert!(header.contains("3 lines"));
    assert!(!header.contains("expand"));

    // Expanded: header drops the line count and has no collapse hint.
    v.toggle_compaction_at(0);
    let flat = v.flatten();
    let header: String = flat[0].spans.iter().map(|s| &*s.content).collect();
    assert!(header.contains("Compaction"));
    assert!(!header.contains("collapse"));
    assert!(!header.contains("lines"), "expanded header has no line count");
}
