use super::super::*;

/// CompactionDelta events stream into an EXPANDED block so the summary is
/// visible while the summarizing LLM call runs. The first delta opens the
/// block; subsequent deltas append to it; the final `Compaction(summary)`
/// event finalizes it without changing its disclosure state.
#[test]
fn compaction_delta_streams_into_expanded_block() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::CompactionDelta("streamed-chunk".into()));

    // The first delta creates exactly one streaming, expanded block.
    assert_eq!(v.blocks.len(), 1, "CompactionDelta must create a block");
    assert!(
        !v.last_compaction_collapsed(),
        "streaming block must be expanded"
    );
    assert!(block_text(&v).contains("streamed-chunk"));

    // A second delta appends to the SAME block (no new block).
    v.apply(&SessionEvent::CompactionDelta(" more".into()));
    assert_eq!(v.blocks.len(), 1);
    assert!(block_text(&v).contains("streamed-chunk more"));

    // The final Compaction event finalizes the block: still expanded, full text.
    v.apply(&SessionEvent::Compaction("final-summary".into()));
    assert_eq!(
        v.blocks.len(),
        1,
        "final Compaction must not add a second block"
    );
    assert!(
        !v.last_compaction_collapsed(),
        "final output must not close the expanded streaming block"
    );
    // The streamed chunks are gone (overwritten), while the final text stays visible.
    assert!(!block_text(&v).contains("streamed-chunk"));
    assert!(block_text(&v).contains("final-summary"));
}

#[test]
fn compaction_updates_preserve_a_user_closed_stream() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::CompactionDelta("first".into()));
    v.toggle_compaction_at(0);
    assert!(v.last_compaction_collapsed(), "user closed the stream");

    v.apply(&SessionEvent::CompactionDelta(" second".into()));
    assert!(
        v.last_compaction_collapsed(),
        "a later delta must not reopen user-closed content"
    );
    v.apply(&SessionEvent::Compaction("final".into()));
    assert!(
        v.last_compaction_collapsed(),
        "final output must preserve the user's closed state"
    );
}

/// When the streaming block was destroyed between deltas and the final event
/// (e.g. by a TranscriptReset replay that rebuilds the chat), the final
/// `Compaction(summary)` creates a fresh collapsed block from scratch.
#[test]
fn compaction_finalizes_without_streaming_block() {
    let mut v = ChatView::default();
    // No prior CompactionDelta block exists.
    v.apply(&SessionEvent::Compaction("final-summary".into()));
    assert_eq!(v.blocks.len(), 1);
    assert!(v.last_compaction_collapsed());
    // Collapsed hides content; expand to verify the summary landed.
    v.toggle_compaction_at(0);
    assert!(block_text(&v).contains("final-summary"));
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
            ChatBlock::Thinking { collapsed, .. } | ChatBlock::Compaction { collapsed, .. } => {
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
    assert!(
        !header.contains("lines"),
        "expanded header has no line count"
    );
}

/// The capitalized header and expanded summary share one exact purple style.
/// BOLD is absent because 16-color terminals may promote it to bright magenta.
#[test]
fn compaction_header_and_text_are_uniform_purple() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::Compaction("summary".into()));

    let collapsed = v.flatten();
    let header: String = collapsed[0]
        .spans
        .iter()
        .map(|span| &*span.content)
        .collect();
    assert!(header.contains("Compaction"));
    assert_eq!(
        collapsed[0].spans[0].style.fg,
        Some(theme::compaction_color())
    );
    assert!(!collapsed[0].spans[0]
        .style
        .add_modifier
        .contains(Modifier::BOLD));

    v.toggle_compaction_at(0);
    let expanded = v.flatten();
    assert_eq!(
        expanded[0].spans[0].style.fg,
        Some(theme::compaction_color())
    );
    assert_eq!(
        expanded[1].spans[0].style.fg,
        Some(theme::compaction_color())
    );
}
