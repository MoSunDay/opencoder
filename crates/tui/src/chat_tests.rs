use super::*;

#[test]
fn text_delta_appends_to_assistant_block() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hello ".into()));
    v.apply(&SessionEvent::TextDelta("world".into()));
    assert!(block_text(&v).contains("hello"));
    assert!(block_text(&v).contains("world"));
}

#[test]
fn reasoning_delta_creates_thinking_block() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("analyzing".into()));
    let flat = v.flatten();
    // Collapsed by default: header shows "Thinking"
    assert!(flat
        .iter()
        .any(|l| { l.spans.iter().any(|s| s.content.contains("Thinking")) }));
    // Content hidden when collapsed
    assert!(!block_text(&v).contains("analyzing"));
    // Expand via block index and verify content
    v.toggle_thinking_at(0);
    assert!(block_text(&v).contains("analyzing"));
}

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
fn done_renders_markdown() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta(
        "# Title\n\nSome **bold** text".into(),
    ));
    v.apply(&SessionEvent::Done);
    // After Done, the assistant block is finalized — check it has rendered
    for b in &v.blocks {
        if let ChatBlock::Assistant { done, .. } = b {
            assert!(*done, "assistant should be finalized after Done");
        }
    }
}

#[test]
fn text_after_tool_starts_fresh_block() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("result:".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "t1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "ls"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "t1".into(),
        name: "bash".into(),
        output: "file1".into(),
        is_error: false,
    });
    v.apply(&SessionEvent::TextDelta("done".into()));
    assert!(block_text(&v).contains("done"));
}

#[test]
fn push_marker_separates_from_assistant() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("streaming".into()));
    v.push_marker(Line::from("[queued] foo"));
    v.apply(&SessionEvent::TextDelta("more".into()));
    assert!(block_text(&v).contains("[queued] foo"));
    assert!(block_text(&v).contains("more"));
}

#[test]
fn agent_switch_updates_agent_without_marker() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::AgentSwitch("act".into()));
    assert_eq!(v.agent, "act");
    assert!(
        !v.blocks.iter().any(|b| matches!(b, ChatBlock::Marker(_))),
        "AgentSwitch must not pollute the chat body with a marker"
    );
}

#[test]
fn agent_switch_finalizes_pending_assistant() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("mid-stream".into()));
    v.apply(&SessionEvent::AgentSwitch("act".into()));
    let pending = v
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Assistant { done, .. } => Some(*done),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!pending.is_empty(), "assistant block should exist");
    assert!(
        pending.iter().all(|d| *d),
        "assistant block must be finalized on AgentSwitch"
    );
}

#[test]
fn multiline_delta_splits_lines() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("line1\nline2".into()));
    let flat = v.flatten();
    let texts: Vec<String> = flat
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    // Assistant text is indented under the `say:` header
    assert!(texts.iter().any(|t| t.contains("line1")), "got {:?}", texts);
    assert!(texts.iter().any(|t| t.contains("line2")), "got {:?}", texts);
}

#[test]
fn error_renders() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::Error("broke".into()));
    assert!(block_text(&v).contains("broke"));
}

#[test]
fn ctx_accumulates_once_at_turn_end_not_per_delta() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hello ".into()));
    v.apply(&SessionEvent::TextDelta("world".into()));
    // Streaming: no per-delta accumulation, so ctx stays at zero and the
    // bottom ctx% bar does not jump on every token.
    assert_eq!(v.context_used, 0, "no accumulation during streaming");
    v.apply(&SessionEvent::Done);
    // Turn boundary: the full assistant text is counted exactly once.
    assert_eq!(v.context_used, estimate("hello world") as u64);
    // Finalizing again must not double-count (idempotent `done` guard).
    v.finalize_assistant();
    assert_eq!(v.context_used, estimate("hello world") as u64);
}

#[test]
fn ctx_counts_reasoning_once_at_finalize() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think ".into()));
    v.apply(&SessionEvent::ReasoningDelta("more".into()));
    assert_eq!(v.context_used, 0, "reasoning not counted while streaming");
    // Reasoning -> text transition seals the thinking block and counts it
    // once, before the assistant text is counted.
    v.apply(&SessionEvent::TextDelta("answer".into()));
    assert_eq!(
        v.context_used,
        estimate("think more") as u64,
        "reasoning counted once on transition; answer not yet counted"
    );
    v.apply(&SessionEvent::Done);
    assert_eq!(
        v.context_used,
        estimate("think more") as u64 + estimate("answer") as u64
    );
    // Re-finalizing must not double-count.
    v.finalize_assistant();
    assert_eq!(
        v.context_used,
        estimate("think more") as u64 + estimate("answer") as u64
    );
}

#[test]
fn paragraph_scroll_uses_wrapped_rows_and_pins_tail() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::{Paragraph, Widget, Wrap};

    let lines: Vec<Line> = vec![
        Line::from("AAAAAAAAAA"),
        Line::from("BBBBBBBBBB"),
        Line::from("CCCCCCCCCCEND"),
    ];
    let width = 10u16;
    let visible_h = 2u16;
    let total_rows = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: false })
        .line_count(width);
    assert_eq!(total_rows, 4);
    let scroll_y = total_rows - visible_h as usize;
    let area = Rect::new(0, 0, width, visible_h);
    let mut buf = Buffer::empty(area);
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll_y as u16, 0))
        .render(area, &mut buf);
    let rs = |y: u16| -> String {
        (0..width)
            .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
            .collect()
    };
    assert!(rs(0).starts_with("CCCCCCCCCC"));
    assert!(rs(visible_h - 1).starts_with("END"));
}

/// Issue #5: with MULTIPLE concurrent subagents, the parent's preamble
/// text is withheld (renders zero lines) and each sibling's completion
/// summary is buffered until the LAST one finishes — so nothing pops in
/// one-by-one. Once all are done, the preamble + every summary surface
/// together.
#[test]
fn multiple_subagents_withhold_output_until_all_done() {
    let mut v = ChatView::default();
    // Parent preamble text precedes the subagent dispatch.
    v.apply(&SessionEvent::TextDelta("launching investigators".into()));
    // Two concurrent subagents (a single one would NOT trigger withholding).
    v.apply(&SessionEvent::SubagentStart {
        id: "a".into(),
        kind: "explore".into(),
        prompt: "p1".into(),
        child_session_id: "ca".into(),
    });
    v.apply(&SessionEvent::SubagentStart {
        id: "b".into(),
        kind: "explore".into(),
        prompt: "p2".into(),
        child_session_id: "cb".into(),
    });

    assert_eq!(v.subagents_running, 2);
    assert!(
        v.hidden_assistant_idx.is_some(),
        "preamble hidden once 2 run"
    );
    assert!(
        !block_text(&v).contains("launching investigators"),
        "preamble withheld while subagents run"
    );

    // First sibling finishes — its summary is buffered, not yet shown.
    v.apply(&SessionEvent::SubagentEnd {
        id: "a".into(),
        ok: true,
        summary: "result-a".into(),
    });
    assert_eq!(v.subagents_running, 1);
    assert_eq!(v.pending_subagent_ends.len(), 1);
    assert!(
        !block_text(&v).contains("result-a"),
        "first summary buffered, not shown while sibling runs"
    );

    // Last sibling finishes — flush everything; preamble + both summaries.
    v.apply(&SessionEvent::SubagentEnd {
        id: "b".into(),
        ok: true,
        summary: "result-b".into(),
    });
    assert_eq!(v.subagents_running, 0);
    assert!(
        v.hidden_assistant_idx.is_none(),
        "preamble revealed once all done"
    );
    let text = block_text(&v);
    assert!(
        text.contains("launching investigators"),
        "preamble reappears"
    );
    assert!(text.contains("result-a"), "first summary shown after flush");
    assert!(
        text.contains("result-b"),
        "second summary shown after flush"
    );
}

/// A SINGLE subagent must NOT trigger withholding: its summary surfaces
/// immediately on its own end, and no preamble is hidden (regression guard
/// for the "multiple only" gate in issue #5).
#[test]
fn single_subagent_does_not_withhold() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("preamble".into()));
    v.apply(&SessionEvent::SubagentStart {
        id: "s".into(),
        kind: "explore".into(),
        prompt: "p".into(),
        child_session_id: "c".into(),
    });
    // Single subagent: never reaches running==2, so no hiding.
    assert!(v.hidden_assistant_idx.is_none());
    assert!(
        block_text(&v).contains("preamble"),
        "preamble still visible"
    );
    // Its summary shows immediately on end (no buffering).
    v.apply(&SessionEvent::SubagentEnd {
        id: "s".into(),
        ok: true,
        summary: "done-single".into(),
    });
    assert!(block_text(&v).contains("done-single"));
    assert!(v.pending_subagent_ends.is_empty());
}

/// Issue #4: a running subagent header renders the animated spinner glyph
/// (one of the SPINNER frames), not the old static dot `\u{25cf}`.
#[test]
fn running_subagent_renders_spinner_not_dot() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::SubagentStart {
        id: "s".into(),
        kind: "explore".into(),
        prompt: "p".into(),
        child_session_id: "c".into(),
    });
    let text0 = block_text_for_tick(&v, 0);
    let text3 = block_text_for_tick(&v, 3);
    // Neither should contain the old static dot.
    assert!(!text0.contains('\u{25cf}'), "no static dot at tick 0");
    assert!(!text3.contains('\u{25cf}'), "no static dot at tick 3");
    // Tick 0 and tick 3 render different spinner frames (it animates).
    assert_ne!(text0, text3, "spinner frame must change with anim_tick");
}

fn block_text_for_tick(v: &ChatView, tick: u32) -> String {
    v.flatten_with(tick)
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect()
}

#[test]
fn parallel_tool_outputs_route_to_own_block() {
    // Regression: when two tools start before either ends (parallel bash
    // calls), each ToolEnd must append output to its own block by id, not to
    // the last-pushed block. Previously all output piled into the final block.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "a".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo A"}),
    });
    v.apply(&SessionEvent::ToolStart {
        id: "b".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo B"}),
    });
    // End out of call order: B finishes first, then A.
    v.apply(&SessionEvent::ToolEnd {
        id: "b".into(),
        name: "bash".into(),
        output: "B-out".into(),
        is_error: false,
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "a".into(),
        name: "bash".into(),
        output: "A-out".into(),
        is_error: false,
    });

    // Two distinct tool blocks, in start order.
    let tools: Vec<_> = v
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Tool { id, header, output } => Some((id, header, output)),
            _ => None,
        })
        .collect();
    assert_eq!(tools.len(), 2, "expected two tool blocks");
    assert_eq!(tools[0].0, "a");
    assert_eq!(tools[1].0, "b");

    let text = |i: usize| -> String {
        tools[i]
            .1
            .spans
            .iter()
            .chain(tools[i].2.iter().flat_map(|l| l.spans.iter()))
            .map(|s| s.content.clone())
            .collect()
    };
    let text_a = text(0);
    let text_b = text(1);

    assert!(text_a.contains("echo A"), "block A header: {text_a}");
    assert!(text_a.contains("A-out"), "block A output: {text_a}");
    assert!(!text_a.contains("B-out"), "block A contaminated: {text_a}");

    assert!(text_b.contains("echo B"), "block B header: {text_b}");
    assert!(text_b.contains("B-out"), "block B output: {text_b}");
    assert!(!text_b.contains("A-out"), "block B contaminated: {text_b}");
}

#[test]
fn orphan_tool_end_creates_synthetic_block() {
    // A ToolEnd with no preceding ToolStart (e.g. a lost event) must not
    // panic; it creates a synthetic "(output)" tool block carrying the id.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolEnd {
        id: "orphan".into(),
        name: "bash".into(),
        output: "loose output".into(),
        is_error: false,
    });
    let tools: Vec<_> = v
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Tool { id, header, output } => Some((id, header, output)),
            _ => None,
        })
        .collect();
    assert_eq!(tools.len(), 1, "orphan ToolEnd should create one block");
    assert_eq!(tools[0].0, "orphan");
    let header: String = tools[0].1.spans.iter().map(|s| s.content.clone()).collect();
    assert!(header.contains("(output)"), "synthetic header: {header}");
    let out: String = tools[0]
        .2
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect();
    assert!(out.contains("loose output"), "output appended: {out}");
}

#[test]
fn tool_end_error_colors_output_red() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "e1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "false"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "e1".into(),
        name: "bash".into(),
        output: "boom".into(),
        is_error: true,
    });
    let tool = v
        .blocks
        .iter()
        .find_map(|b| match b {
            ChatBlock::Tool { output, .. } => Some(output),
            _ => None,
        })
        .expect("tool block");
    assert!(!tool.is_empty(), "error output should be appended");
    assert_eq!(
        tool[0].spans[0].style.fg,
        Some(ratatui::style::Color::Red),
        "error output must be styled red"
    );
}

#[test]
fn tool_output_truncated_to_six_lines() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "seq 20"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "t1".into(),
        name: "bash".into(),
        output: (1..=20)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        is_error: false,
    });
    let tool = v
        .blocks
        .iter()
        .find_map(|b| match b {
            ChatBlock::Tool { output, .. } => Some(output),
            _ => None,
        })
        .expect("tool block");
    assert_eq!(
        tool.len(),
        6,
        "output must be truncated to TOOL_OUTPUT_LINES (6); got {}",
        tool.len()
    );
}
