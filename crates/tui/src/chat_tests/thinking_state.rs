use super::super::*;

#[test]
fn thinking_block_collapses() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("line1\nline2\nline3".into()));
    // Collapsed by default: summary line only, content hidden
    let text = block_text(&v);
    assert!(text.contains("3 lines"));
    assert!(!text.contains("line1"));
    // Expand: should contain all 3 lines
    v.toggle_thinking_at(0);
    assert!(block_text(&v).contains("line1"));
    assert!(block_text(&v).contains("line3"));
    // Collapse again
    v.toggle_thinking_at(0);
    assert!(!block_text(&v).contains("line1"));
}

#[test]
fn thinking_headers_match_flatten_line_indices() {
    let mut v = ChatView::default();
    // Two thinking blocks separated by an assistant block.
    v.apply(&SessionEvent::ReasoningDelta("think-a".into()));
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::Done);
    v.apply(&SessionEvent::ReasoningDelta("think-b-1\nthink-b-2".into()));

    let flat = v.flatten();
    let headers = v.thinking_headers();
    assert_eq!(headers.len(), 2, "expected two thinking headers");
    // Each recorded header line must contain the "Thinking" header text.
    for h in &headers {
        let line = &flat[h.header_line_idx];
        assert!(
            line.spans.iter().any(|s| s.content.contains("Thinking")),
            "header_line_idx {} is not a Thinking header: {:?}",
            h.header_line_idx,
            line,
        );
    }
    // block_idx maps back to a Thinking block.
    for h in &headers {
        assert!(
            matches!(v.blocks[h.block_idx], ChatBlock::Thinking { .. }),
            "block_idx {} is not a Thinking block",
            h.block_idx,
        );
    }
    // Expanding the second block shifts nothing before it; first header
    // line index is unchanged.
    let first_before = headers[0].header_line_idx;
    v.toggle_thinking_at(headers[1].block_idx);
    let first_after = v.thinking_headers()[0].header_line_idx;
    assert_eq!(first_before, first_after);
}

#[test]
fn toggle_thinking_at_toggles_specific_block() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("first".into()));
    v.apply(&SessionEvent::TextDelta("between".into()));
    v.apply(&SessionEvent::Done);
    v.apply(&SessionEvent::ReasoningDelta("second".into()));

    let headers = v.thinking_headers();
    assert_eq!(headers.len(), 2);
    // Both collapsed initially.
    assert!(!block_text(&v).contains("first"));
    assert!(!block_text(&v).contains("second"));
    // Toggle only the first: its content shows, second stays hidden.
    v.toggle_thinking_at(headers[0].block_idx);
    assert!(block_text(&v).contains("first"));
    assert!(!block_text(&v).contains("second"));
    // Out-of-range / non-thinking index is a no-op.
    v.toggle_thinking_at(999);
    v.toggle_thinking_at(headers[0].block_idx + 1); // assistant block index
    assert!(block_text(&v).contains("first"));
}

#[test]
fn collapse_all_collapsible_collapses_every_thinking_block() {
    let mut v = ChatView::default();
    // Two thinking blocks separated by an assistant block.
    v.apply(&SessionEvent::ReasoningDelta("think-a".into()));
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::Done);
    v.apply(&SessionEvent::ReasoningDelta("think-b\nthink-c".into()));

    let headers = v.thinking_headers();
    assert_eq!(headers.len(), 2);
    // Expand both so they are observably NOT collapsed.
    for h in &headers {
        v.toggle_thinking_at(h.block_idx);
    }
    assert!(block_text(&v).contains("think-a"));
    assert!(block_text(&v).contains("think-b"));

    // Collapse all in one call.
    v.collapse_all_collapsible();

    // Every Thinking block is collapsed, regardless of sealed state.
    for b in &v.blocks {
        if let ChatBlock::Thinking { collapsed, .. } = b {
            assert!(*collapsed, "thinking block must be collapsed");
        }
    }
    // Content is hidden again once collapsed.
    assert!(!block_text(&v).contains("think-a"));
    assert!(!block_text(&v).contains("think-b"));
}

#[test]
fn collapse_all_collapsible_noop_without_collapsible_blocks() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("just text".into()));
    v.apply(&SessionEvent::Done);
    // No Thinking blocks present: must not panic and leaves state intact.
    v.collapse_all_collapsible();
    assert!(block_text(&v).contains("just text"));
}

#[test]
fn last_thinking_collapsed_empty_view() {
    let view = ChatView::default();
    assert!(!view.last_thinking_collapsed());
}

#[test]
fn last_thinking_collapsed_true_when_collapsed() {
    let mut view = ChatView::default();
    view.apply(&SessionEvent::ReasoningDelta("thinking...".into()));
    assert!(view.last_thinking_collapsed());
}

#[test]
fn last_thinking_collapsed_false_when_expanded() {
    let mut view = ChatView::default();
    view.apply(&SessionEvent::ReasoningDelta("thinking...".into()));
    // Toggle expands the (only) thinking block at index 0.
    view.toggle_thinking_at(0);
    assert!(!view.last_thinking_collapsed());
}

#[test]
fn last_thinking_collapsed_false_when_last_block_not_thinking() {
    let mut view = ChatView::default();
    view.apply(&SessionEvent::ReasoningDelta("thinking...".into()));
    // A TextDelta seals the thinking block and opens an assistant block.
    view.apply(&SessionEvent::TextDelta("answer".into()));
    assert!(!view.last_thinking_collapsed());
}
