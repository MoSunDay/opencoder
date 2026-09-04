use super::super::*;

/// The top-level Thinking block is a legacy/replay shape (live reasoning
/// streams into the step ladder) — these tests build it directly to keep the
/// block machinery (collapse/toggle/headers) covered.
fn legacy_thinking_view(runs: &[(&str, bool)]) -> ChatView {
    let mut v = ChatView::default();
    for &(text, sealed) in runs {
        v.blocks.push(ChatBlock::Thinking {
            text: text.into(),
            collapsed: true,
            sealed,
        });
    }
    v
}

#[test]
fn thinking_block_collapses() {
    let mut v = legacy_thinking_view(&[("line1\nline2\nline3", false)]);
    // Collapsed by default: header only, content hidden
    let text = block_text(&v);
    assert!(text.contains("Thinking"));
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
    // Two thinking blocks separated by an assistant block.
    let mut v = legacy_thinking_view(&[("think-a", false)]);
    // Built directly: a streamed Say would close its turn and fold the
    // pending thinking into the ladder, but these tests target the legacy
    // top-level Thinking block machinery itself.
    v.blocks.push(ChatBlock::Assistant {
        raw: "hi".into(),
        rendered: Vec::new(),
        done: true,
    });
    v.blocks.push(ChatBlock::Thinking {
        text: "think-b-1\nthink-b-2".into(),
        collapsed: true,
        sealed: false,
    });

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
    // Legacy shape: two Thinking blocks separated by an assistant, built
    // directly (live reasoning goes into the step ladder, and a streamed
    // Say would close its turn and fold the pending thinking away).
    let mut v = legacy_thinking_view(&[("first run", false)]);
    v.blocks.push(ChatBlock::Assistant {
        raw: "between".into(),
        rendered: Vec::new(),
        done: true,
    });
    v.blocks.push(ChatBlock::Thinking {
        text: "second run".into(),
        collapsed: true,
        sealed: false,
    });

    let headers = v.thinking_headers();
    assert_eq!(headers.len(), 2);
    // Both collapsed initially.
    assert!(!block_text(&v).contains("first run"));
    assert!(!block_text(&v).contains("second run"));
    // Toggle only the first: its content shows, second stays hidden.
    v.toggle_thinking_at(headers[0].block_idx);
    assert!(block_text(&v).contains("first run"));
    assert!(!block_text(&v).contains("second run"));
    // Out-of-range / non-thinking index is a no-op.
    v.toggle_thinking_at(999);
    v.toggle_thinking_at(headers[0].block_idx + 1); // assistant block index
    assert!(block_text(&v).contains("first run"));
}

#[test]
fn collapse_all_collapsible_collapses_every_thinking_block() {
    // Two thinking blocks separated by an assistant block.
    let mut v = legacy_thinking_view(&[("think-a", false)]);
    // Built directly: a streamed Say would close its turn and fold the
    // pending thinking into the ladder (see `append_text_delta`).
    v.blocks.push(ChatBlock::Assistant {
        raw: "hi".into(),
        rendered: Vec::new(),
        done: true,
    });
    v.blocks.push(ChatBlock::Thinking {
        text: "think-b\nthink-c".into(),
        collapsed: true,
        sealed: false,
    });

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
fn last_open_thinking_collapsed_empty_view() {
    let view = ChatView::default();
    assert!(!view.last_open_thinking_collapsed());
}

#[test]
fn last_open_thinking_collapsed_true_when_collapsed() {
    let view = legacy_thinking_view(&[("thinking...", false)]);
    assert!(view.last_open_thinking_collapsed());
}

#[test]
fn last_open_thinking_collapsed_false_when_expanded() {
    let mut view = legacy_thinking_view(&[("thinking...", false)]);
    // Toggle expands the (only) thinking block at index 0.
    view.toggle_thinking_at(0);
    assert!(!view.last_open_thinking_collapsed());
}

#[test]
fn last_open_thinking_collapsed_false_when_last_block_not_thinking() {
    let mut view = legacy_thinking_view(&[("thinking...", true)]);
    // A sealed thinking block followed by an assistant block.
    view.apply(&SessionEvent::TextDelta("answer".into()));
    assert!(!view.last_open_thinking_collapsed());
}

#[test]
fn last_open_thinking_collapsed_false_when_sealed() {
    let view = legacy_thinking_view(&[("thinking...", true)]);
    assert!(!view.last_open_thinking_collapsed());
}

/// Collapsed Thinking header shows the icon + label and the `(N lines)` count;
/// expanded header drops the count. This guards the line-count summary that was
/// accidentally removed from the shared `render_collapsible`.
#[test]
fn thinking_header_shows_line_count_when_collapsed() {
    // Body is 4 lines.
    let mut v = legacy_thinking_view(&[("l1\nl2\nl3\nl4", false)]);

    // Collapsed: header carries the line count.
    let flat = v.flatten();
    let header: String = flat[0].spans.iter().map(|s| &*s.content).collect();
    assert!(header.contains("Thinking"), "collapsed header has label");
    assert!(
        header.contains("4 lines"),
        "collapsed header shows line count"
    );
    // Content is hidden while collapsed.
    assert!(!header.contains("l1"));

    // Expanded: header no longer carries the line count.
    v.toggle_thinking_at(0);
    let flat = v.flatten();
    let header: String = flat[0].spans.iter().map(|s| &*s.content).collect();
    assert!(header.contains("Thinking"), "expanded header has label");
    assert!(
        !header.contains("lines"),
        "expanded header must not carry line count"
    );
}

#[test]
fn interleaved_reasoning_opens_a_new_turn_under_each_say() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::LlmRoundStart { started_at_ms: 1 });
    v.apply(&SessionEvent::ReasoningDelta("plan regression".into()));
    v.apply(&SessionEvent::TextDelta("全量回归".into()));
    v.apply(&SessionEvent::ReasoningDelta("record totals".into()));
    v.apply(&SessionEvent::TextDelta(
        "通过，无失败。统计总数并写 changelog。".into(),
    ));

    // Contract: `1 Turn = n Steps + Say`; one submission may hold several
    // turns. The first Say closes turn 1 ("plan regression" is ITS ladder);
    // the reasoning after it opens turn 2's ladder BELOW the Say and the
    // second Say closes that turn. Nothing merges across a closed Say.
    let thinking_of = |v: &ChatView, group_idx: usize| match &v.blocks[group_idx] {
        ChatBlock::StepGroup { steps, .. } => steps[0].thinking_raw.clone(),
        _ => unreachable!("block {group_idx} must be a step group"),
    };
    assert!(matches!(v.blocks[0], ChatBlock::StepGroup { .. }));
    assert!(matches!(v.blocks[1], ChatBlock::Assistant { .. }));
    assert!(matches!(v.blocks[2], ChatBlock::StepGroup { .. }));
    assert!(matches!(v.blocks[3], ChatBlock::Assistant { .. }));
    assert_eq!(thinking_of(&v, 0), "plan regression");
    assert_eq!(thinking_of(&v, 2), "record totals");
    let assistants: Vec<_> = v
        .blocks
        .iter()
        .filter_map(|block| match block {
            ChatBlock::Assistant { raw, .. } => Some(raw.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        assistants,
        ["全量回归", "通过，无失败。统计总数并写 changelog。"]
    );

    // Each Say merges into its preceding group: `{glyph} Say(n step{s}):`
    // — one merged header per turn pair.
    let say_headers = v
        .flatten()
        .iter()
        .filter(|line| line.spans.iter().any(|span| span.content.contains("Say(")))
        .count();
    assert_eq!(
        say_headers, 2,
        "each Say of the pairing contract renders its own merged header"
    );
}

#[test]
fn collapsed_live_reasoning_stays_raw_until_the_step_opens() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("first".into()));
    v.apply(&SessionEvent::TextDelta("answer".into()));
    v.apply(&SessionEvent::ReasoningDelta("second".into()));

    // The Say in between closed turn 1 ("first" is ITS sealed ladder) and
    // opened turn 2 BELOW it, so "second" streams into the new group at
    // blocks[2] — never back into the closed turn's step.
    assert!(matches!(v.blocks[1], ChatBlock::Assistant { .. }));
    assert!(matches!(v.blocks[2], ChatBlock::StepGroup { .. }));

    // The trailing step is structurally present but hidden, so deltas only
    // append raw source and the render loop may skip a delta-only frame.
    assert!(
        v.last_open_thinking_collapsed(),
        "collapsed step reasoning is not visible"
    );
    let (thinking_raw, thinking_rendered) = match &v.blocks[0] {
        ChatBlock::StepGroup { steps, .. } => (
            steps[0].thinking_raw.clone(),
            crate::chat::steps::span_text(&steps[0].thinking),
        ),
        _ => unreachable!("first block must be the step group"),
    };
    assert_eq!(thinking_raw, "first");
    // Closing the Say sealed turn 1's step: its thinking is rendered then
    // and there (counted exactly once), so the closed turn carries the
    // rendered body already.
    assert_eq!(thinking_rendered, "first");
    let (raw2, empty2) = match &v.blocks[2] {
        ChatBlock::StepGroup { steps, .. } => {
            (steps[0].thinking_raw.clone(), steps[0].thinking.is_empty())
        }
        _ => unreachable!("third block must be the step group"),
    };
    assert_eq!(raw2, "second");
    assert!(empty2);

    v.toggle_tool_call_at(2, 0);
    v.toggle_tool_call_at(2, 1);
    let thinking = match &v.blocks[2] {
        ChatBlock::StepGroup { steps, .. } => crate::chat::steps::span_text(&steps[0].thinking),
        _ => unreachable!("third block must be the step group"),
    };
    assert_eq!(thinking, "second");
}

#[test]
fn interleaved_round_finalization_counts_once_and_hard_bounds_next_round() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think-a".into()));
    v.apply(&SessionEvent::TextDelta("answer-a".into()));
    v.apply(&SessionEvent::ReasoningDelta("think-b".into()));
    v.apply(&SessionEvent::LlmRoundEnd);

    let expected =
        estimate("think-a") as u64 + estimate("think-b") as u64 + estimate("answer-a") as u64;
    assert_eq!(v.context_used, expected);
    // The Say closed its Turn and the reasoning that followed opened the
    // NEXT turn's ladder below it: the last block is that ladder, and the
    // Say itself (blocks[1]) is the one finalized by this round end.
    assert!(matches!(v.blocks.last(), Some(ChatBlock::StepGroup { .. })));
    assert!(matches!(
        v.blocks.get(1),
        Some(ChatBlock::Assistant { done: true, .. })
    ));

    v.apply(&SessionEvent::LlmRoundStart { started_at_ms: 2 });
    v.apply(&SessionEvent::TextDelta("answer-b".into()));
    assert_eq!(
        v.blocks
            .iter()
            .filter(|block| matches!(block, ChatBlock::Assistant { .. }))
            .count(),
        2,
        "a new LLM round is a hard Assistant merge boundary"
    );

    v.apply(&SessionEvent::Done);
    assert_eq!(
        v.context_used,
        expected + estimate("answer-b") as u64,
        "finalizing again must not double-count the first round"
    );
}

#[test]
fn completed_answer_repairs_dropped_chunks_without_touching_previous_turn() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("old answer".into()));
    v.apply(&SessionEvent::Done);

    v.begin_turn();
    v.apply(&SessionEvent::ReasoningDelta("new thinking".into()));
    v.apply(&SessionEvent::TextDelta("全量回归".into()));
    v.apply(&SessionEvent::LlmRoundEnd);
    v.apply(&SessionEvent::Done);
    v.reconcile_completed_assistant("全量回归通过，无失败。");

    let assistants: Vec<_> = v
        .blocks
        .iter()
        .filter_map(|block| match block {
            ChatBlock::Assistant { raw, .. } => Some(raw.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(assistants, ["old answer", "全量回归通过，无失败。"]);
    // Turn one is pure text (standalone `❯ Say:` header); the repaired turn
    // has a ladder (merged `Say(n steps)` header) — exactly one Say per turn.
    let count_say_rows = |v: &ChatView| {
        v.flatten()
            .iter()
            .filter(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains("Say(") || span.content.contains("Say:"))
            })
            .count()
    };
    assert_eq!(
        count_say_rows(&v),
        2,
        "one Say per turn remains after completed-text repair"
    );
}

#[test]
fn completed_answer_creates_say_when_every_text_delta_was_dropped() {
    let mut v = ChatView::default();
    v.begin_turn();
    v.apply(&SessionEvent::ReasoningDelta("thinking".into()));
    v.apply(&SessionEvent::Done);
    v.reconcile_completed_assistant("recovered answer");

    // The pending Thinking was flushed into a call-less step at Done; the
    // recovered Say lands AFTER the ladder (it is the turn's conclusion).
    assert!(matches!(
        v.blocks[0],
        ChatBlock::StepGroup { ref steps, .. } if !steps.is_empty()
            && !steps[0].thinking.is_empty()
            && steps[0].calls.is_empty()
    ));
    assert!(matches!(
        v.blocks[1],
        ChatBlock::Assistant {
            ref raw,
            done: true,
            ..
        } if raw == "recovered answer"
    ));
    // A StepGroup self-terminates with its own trailing blank, so Done must
    // NOT stack a boundary marker after it ("exactly one blank after the
    // turn"): the recovered Say is the turn's final block.
    assert_eq!(
        v.blocks.len(),
        2,
        "no boundary marker after a self-terminating StepGroup: {:?}",
        v.blocks
    );
    assert_eq!(
        v.context_used,
        estimate("thinking") as u64 + estimate("recovered answer") as u64
    );
}
