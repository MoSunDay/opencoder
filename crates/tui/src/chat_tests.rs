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
    // Verify markdown was actually rendered (not just the done flag): the H1
    // heading and **bold** carry Modifier::BOLD, which plain-text streaming
    // (done=false) never applies. Exclude the "say:" header which is always
    // bold regardless of rendering state.
    let has_md_bold = v.flatten().iter().any(|line| {
        line.spans.iter().any(|s| {
            s.style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
                && (s.content.contains("Title") || s.content.contains("bold"))
        })
    });
    assert!(
        has_md_bold,
        "flattened output should contain markdown-rendered BOLD spans after Done"
    );
}

#[test]
fn finalize_assistant_idempotent() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hello **world**".into()));
    v.apply(&SessionEvent::Done);
    // Capture full state after the first finalize (Done triggers it).
    let before = v.clone();
    let ctx = v.context_used;
    // Finalize again — must be a complete no-op.
    v.finalize_assistant();
    assert_eq!(v, before, "second finalize_assistant must not change state");
    assert_eq!(v.context_used, ctx, "context_used must not double-count");
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
        images: Vec::new(),
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
fn plan_submitted_defaults_false() {
    let v = ChatView::default();
    assert!(
        !v.plan_submitted,
        "plan_submitted must default to false so a fresh session never \
         triggers the plan->act handoff spuriously"
    );
}

#[test]
fn agent_switch_to_plan_resets_plan_submitted() {
    // Regression: switching into plan mode must reset the flag so that the
    // plan->act handoff only fires when the user actually submitted a prompt
    // during THIS plan session. Previously the check used
    // !chat.blocks.is_empty(), which is always true (blocks hold act history),
    // causing an accidental plan->act toggle to collapse the transcript.
    let mut v = ChatView {
        plan_submitted: true,
        ..Default::default()
    };
    v.apply(&SessionEvent::AgentSwitch("plan".into()));
    assert!(
        !v.plan_submitted,
        "entering plan mode must reset plan_submitted to false"
    );
}

#[test]
fn agent_switch_to_act_keeps_plan_submitted() {
    // Switching to act must NOT reset the flag — the app.rs event loop reads
    // it BEFORE the AgentSwitch event arrives to decide handoff vs plain swap.
    let mut v = ChatView {
        plan_submitted: true,
        ..Default::default()
    };
    v.apply(&SessionEvent::AgentSwitch("act".into()));
    assert!(
        v.plan_submitted,
        "switching to act must not clobber plan_submitted"
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
fn streaming_trailing_newline_does_not_add_blank_line() {
    let mut v = ChatView::default();
    // A stream chunk ending in a newline must not render an extra trailing
    // blank body line — consistency with `flush_code` in markdown.rs.
    v.apply(&SessionEvent::TextDelta("only\n".into()));
    let joined: Vec<String> = v
        .flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    // The final flattened line is the last assistant body line; it must carry
    // real content, not be a bare indent.
    let last = joined.last().expect("at least one line");
    assert!(
        last.trim_end().contains("only"),
        "trailing newline must not add an empty body line; got {:?}",
        joined
    );
}

#[test]
fn streaming_interior_blank_line_is_preserved() {
    let mut v = ChatView::default();
    // An interior blank line is genuine content and must be kept — only the
    // single *trailing* empty split element is dropped.
    v.apply(&SessionEvent::TextDelta("a\n\nb".into()));
    let joined: Vec<String> = v
        .flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    // Exactly two content lines (a, b) plus one interior indent-only blank line.
    let content: Vec<&String> = joined
        .iter()
        .filter(|t| t.trim_end().ends_with('a') || t.trim_end().ends_with('b'))
        .collect();
    assert_eq!(
        content.len(),
        2,
        "expected two content lines; got {:?}",
        joined
    );
    let interior_blank = joined.iter().filter(|t| t.trim().is_empty()).count();
    assert_eq!(
        interior_blank, 1,
        "expected one interior blank line; got {:?}",
        joined
    );
    let last = joined.last().expect("at least one line");
    assert!(
        last.trim_end().contains('b'),
        "no trailing blank expected; got {:?}",
        joined
    );
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
    // status bar's ctx% indicator does not jump on every token.
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
        cancelled: false,
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
        cancelled: false,
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
        cancelled: false,
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
        images: Vec::new(),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "a".into(),
        name: "bash".into(),
        output: "A-out".into(),
        is_error: false,
        images: Vec::new(),
    });

    // Two distinct tool blocks, in start order.
    let tools: Vec<_> = v
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Tool { id, header, output, .. } => Some((id, header, output)),
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
        images: Vec::new(),
    });
    let tools: Vec<_> = v
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Tool { id, header, output, .. } => Some((id, header, output)),
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
        images: Vec::new(),
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
fn tool_output_retained_in_full_and_collapsed_by_default() {
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
        images: Vec::new(),
    });
    let (output, collapsed) = v
        .blocks
        .iter()
        .find_map(|b| match b {
            ChatBlock::Tool { output, collapsed, .. } => Some((output, *collapsed)),
            _ => None,
        })
        .expect("tool block");
    // No truncation: all 20 lines are retained.
    assert_eq!(
        output.len(),
        20,
        "full output must be retained (was truncated to 6); got {}",
        output.len()
    );
    // Tool blocks start collapsed by default.
    assert!(collapsed, "tool block must default to collapsed");
}

#[test]
fn toggle_tool_at_expands_then_collapses() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo hi"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "t1".into(),
        name: "bash".into(),
        output: "RESULT-42".into(),
        is_error: false,
        images: Vec::new(),
    });
    assert!(
        matches!(v.blocks.last(), Some(ChatBlock::Tool { collapsed: true, .. })),
        "tool block should start collapsed"
    );
    // While collapsed, the output body must be hidden from flatten().
    let flat_collapsed = v.flatten();
    let body: String = flat_collapsed
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect();
    assert!(
        !body.contains("RESULT-42"),
        "collapsed tool must hide its output; got: {body:?}"
    );

    let idx = v.blocks.len() - 1;
    v.toggle_tool_at(idx);
    let flat_expanded = v.flatten();
    let body2: String = flat_expanded
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect();
    assert!(
        body2.contains("RESULT-42"),
        "expanded tool must show its output; got: {body2:?}"
    );
    assert!(
        flat_expanded.len() > flat_collapsed.len(),
        "expanded must render more lines than collapsed"
    );

    // Toggle back to collapsed.
    v.toggle_tool_at(idx);
    assert!(
        matches!(v.blocks.last(), Some(ChatBlock::Tool { collapsed: true, .. })),
        "second toggle must re-collapse"
    );
}

#[test]
fn toggle_tool_at_is_noop_for_non_tool_blocks() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hello".into()));
    v.apply(&SessionEvent::Done);
    // Index 0 is an Assistant block, not a Tool — toggling must be a no-op.
    v.toggle_tool_at(0);
    assert!(block_text(&v).contains("hello"), "non-tool toggle must not corrupt state");
}

#[test]
fn collapse_all_collapsible_collapses_tools_and_thinking() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("reason".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "t".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "ls"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "t".into(),
        name: "bash".into(),
        output: "out".into(),
        is_error: false,
        images: Vec::new(),
    });
    // Expand both so they are observably NOT collapsed beforehand.
    for h in v.thinking_headers() {
        v.toggle_thinking_at(h.block_idx);
    }
    for h in v.tool_headers() {
        v.toggle_tool_at(h.block_idx);
    }
    v.collapse_all_collapsible();
    for b in &v.blocks {
        match b {
            ChatBlock::Thinking { collapsed, .. } | ChatBlock::Tool { collapsed, .. } => {
                assert!(*collapsed, "every collapsible block must be collapsed");
            }
            _ => {}
        }
    }
}

#[test]
fn tool_headers_line_index_lands_on_tool_header() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("preamble\nsecond".into()));
    v.apply(&SessionEvent::Done);
    v.apply(&SessionEvent::ToolStart {
        id: "t".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    let headers = v.tool_headers();
    assert_eq!(headers.len(), 1, "expected exactly one tool header");
    let flat = v.flatten();
    let header_line: String = flat[headers[0].header_line_idx]
        .spans
        .iter()
        .map(|s| s.content.clone())
        .collect();
    assert!(
        header_line.contains("bash"),
        "header_line_idx must land on the tool header line; got: {header_line:?}"
    );
}

#[test]
fn summarize_keeps_full_bash_command_no_truncation() {
    // Regression: bash commands longer than 80 columns were truncated to
    // 80 display columns (with …), hiding the real command behind an
    // ellipsis. summarize() must now return the full command text so the
    // body layer can wrap it to the terminal width.
    let long_cmd = format!("echo {}", "a".repeat(100));
    assert!(
        long_cmd.chars().count() > 80,
        "test setup: command must exceed 80 cols"
    );
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": long_cmd.clone()}),
    });
    let header = v
        .blocks
        .iter()
        .find_map(|b| match b {
            ChatBlock::Tool { header, .. } => Some(header),
            _ => None,
        })
        .expect("tool block");
    // spans[0] is the "▸ bash " label; spans[1] is the summarize() output.
    let summary = header.spans[1].content.to_string();
    assert!(
        summary.contains(&long_cmd),
        "header must contain the full command; got {summary:?}"
    );
    assert!(
        !summary.contains('\u{2026}'),
        "header must not be truncated with ellipsis; got {summary:?}"
    );
}

#[test]
fn collapse_all_thinking_collapses_every_block() {
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
fn collapse_all_thinking_noop_without_thinking_blocks() {
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

#[test]
fn short_truncates_by_display_width_not_char_count() {
    // Ten CJK characters = 20 terminal columns. A budget of 10 must be
    // interpreted as 10 columns, so the result never exceeds 10 columns.
    // With the old char-count logic this returned 10 chars (20 cols) + "...".
    let wide = "你好世界测试你好世界";
    let out = short(wide, 10);
    assert!(
        composer::str_width(&out) <= 10,
        "short() must fit in 10 columns; got {out:?} ({} cols)",
        composer::str_width(&out)
    );
    assert!(
        out.ends_with('…'),
        "truncated output should end with ellipsis; got {out:?}"
    );

    // Short strings are returned unchanged.
    assert_eq!(short("hi", 10), "hi");
    // Long ASCII is also bounded to the display-width budget.
    let long_ascii = short("abcdefghijklmnopqrstuvwxyz", 10);
    assert!(composer::str_width(&long_ascii) <= 10);
    assert!(long_ascii.ends_with('…'));
}

#[test]
fn plan_handoff_creates_plan_card() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::PlanHandoff(
        "## Plan\n1. do X\n2. do Y".into(),
    ));

    // A Plan block is pushed.
    assert!(
        v.blocks.iter().any(|b| matches!(b, ChatBlock::Plan { .. })),
        "PlanHandoff must create a Plan block"
    );

    // The card renders with a header and the markdown content.
    let flat = v.flatten();
    let text: String = flat
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
        .collect();
    assert!(text.contains("plan"), "plan header must be present");
    assert!(text.contains("Plan"), "plan heading text must be present");
    assert!(text.contains("do X"), "plan content must be present");
    assert!(
        !text.contains("## Plan"),
        "heading markup must be rendered, not raw"
    );
}

#[test]
fn plan_handoff_finalizes_pending_assistant() {
    // An in-progress assistant block must be finalized before the Plan card
    // is pushed, so the plan appears as a separate block.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("partial response".into()));
    v.apply(&SessionEvent::PlanHandoff("## Plan".into()));

    let assistant_count = v
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Assistant { .. }))
        .count();
    assert_eq!(assistant_count, 1, "assistant block must be finalized");
    assert!(
        v.blocks
            .last()
            .map(|b| matches!(b, ChatBlock::Plan { .. }))
            .unwrap_or(false),
        "Plan block must be last"
    );
}

#[test]
fn plan_card_line_count_matches_flatten() {
    // Verify thinking_headers/subagent_headers line counting stays aligned
    // when a Plan block precedes a Thinking block.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::PlanHandoff("line one\nline two".into()));
    v.apply(&SessionEvent::ReasoningDelta("think".into()));

    let flat = v.flatten();
    let headers = v.thinking_headers();
    assert_eq!(headers.len(), 1, "one thinking header expected");
    let line = &flat[headers[0].header_line_idx];
    assert!(
        line.spans.iter().any(|s| s.content.contains("Thinking")),
        "thinking header must point at the correct line"
    );
}

#[test]
fn plan_card_flatten_structure() {
    use ratatui::style::{Color, Modifier};

    let mut v = ChatView::default();
    v.apply(&SessionEvent::PlanHandoff("## Goal\nShip it".into()));

    let flat = v.flatten();

    // Line 0: Yellow bold header "── plan ──".
    let header = &flat[0];
    assert!(
        header.spans.iter().any(|s| s.content.contains("plan")),
        "first line must be the plan header, got: {:?}",
        header.spans
    );
    // Verify the Yellow + Bold styling on the header span.
    assert!(
        header.spans.iter().any(|s| {
            s.style.fg == Some(Color::Yellow) && s.style.add_modifier.contains(Modifier::BOLD)
        }),
        "plan header must be Yellow + Bold"
    );

    // Body lines are indented (start with 2 spaces).
    let body_line = &flat[1];
    assert!(
        body_line
            .spans
            .first()
            .map(|s| s.content.starts_with("  "))
            .unwrap_or(false),
        "body lines must be indented by 2 spaces, got: {:?}",
        body_line.spans
    );

    // Trailing blank line after the body.
    assert!(
        flat.last().map(|l| l.spans.is_empty()).unwrap_or(false),
        "Plan card must end with a trailing blank line"
    );
}

#[test]
fn begin_turn_clears_status() {
    // A transient status set on the previous turn (e.g. an interrupted marker
    // surfaced via SessionEvent::Status) must be cleared at the start of the
    // next turn so it does not leak into the status bar.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::Status("interrupted".into()));
    assert_eq!(v.status, "interrupted");
    v.begin_turn();
    assert!(
        v.status.is_empty(),
        "begin_turn must clear transient status"
    );
}

#[test]
fn begin_turn_preserves_transcript() {
    // The turn-start invariant only clears presentation status — the
    // transcript blocks must be untouched.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hello world".into()));
    v.apply(&SessionEvent::Status("interrupted".into()));
    let before = block_text(&v);
    v.begin_turn();
    assert_eq!(
        block_text(&v),
        before,
        "transcript blocks must survive begin_turn"
    );
    assert!(v.status.is_empty());
}

#[test]
fn steer_consumed_pushes_marker_and_drops_entry() {
    // When a steer is promoted at the turn boundary, the view embeds a
    // `steer: {prompt}` marker into the transcript (so the user sees WHEN it
    // took effect) and drops the pending entry by seq.
    let mut v = ChatView::default();
    v.steer_items.push((7, "use python".into()));
    v.apply(&SessionEvent::SteerConsumed { seq: 7 });
    assert!(
        block_text(&v).contains("steer: use python"),
        "SteerConsumed must embed a steer marker with the prompt text"
    );
    assert!(
        v.steer_items.is_empty(),
        "SteerConsumed must drop the consumed entry from steer_items"
    );
}

#[test]
fn steer_consumed_unknown_seq_is_noop() {
    // A SteerConsumed whose seq does not match any pending entry must be a
    // no-op: no marker is pushed and the existing entries are retained.
    let mut v = ChatView::default();
    v.steer_items.push((7, "use python".into()));
    let before = block_text(&v);
    v.apply(&SessionEvent::SteerConsumed { seq: 999 });
    assert_eq!(block_text(&v), before, "unknown seq must not push a marker");
    assert_eq!(
        v.steer_items.len(),
        1,
        "unknown seq must retain all entries"
    );
}

#[test]
fn last_plan_text_returns_raw_from_plan_block() {
    // When a Plan block exists, last_plan_text must return its `raw` field
    // (the editable markdown source), ignoring any Assistant blocks.
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::Plan {
        rendered: crate::markdown::render("## Plan\n- step one"),
        raw: "## Plan\n- step one".to_string(),
    });
    assert_eq!(
        v.last_plan_text().as_deref(),
        Some("## Plan\n- step one"),
        "last_plan_text must return the Plan block's raw field"
    );
}

#[test]
fn last_plan_text_falls_back_to_assistant_raw() {
    // With no Plan block, last_plan_text falls back to the last non-empty
    // Assistant block's raw — in plan mode the plan IS the last assistant
    // message before the Plan card is handed off.
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::Assistant {
        raw: "first reply".to_string(),
        rendered: crate::markdown::render("first reply"),
        done: true,
    });
    v.blocks.push(ChatBlock::Assistant {
        raw: "second reply".to_string(),
        rendered: crate::markdown::render("second reply"),
        done: true,
    });
    assert_eq!(
        v.last_plan_text().as_deref(),
        Some("second reply"),
        "with no Plan block, last_plan_text must return the last non-empty Assistant raw"
    );
}

#[test]
fn last_plan_text_skips_empty_assistant() {
    // An empty Assistant block must be skipped in favour of the most recent
    // non-empty one.
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::Assistant {
        raw: String::new(),
        rendered: Vec::new(),
        done: false,
    });
    v.blocks.push(ChatBlock::Assistant {
        raw: "real content".to_string(),
        rendered: crate::markdown::render("real content"),
        done: true,
    });
    assert_eq!(
        v.last_plan_text().as_deref(),
        Some("real content"),
        "last_plan_text must skip the empty Assistant and return the non-empty one"
    );
}

#[test]
fn last_plan_text_returns_none_when_empty() {
    // An empty ChatView has nothing to return.
    let v = ChatView::default();
    assert!(
        v.last_plan_text().is_none(),
        "last_plan_text must be None for an empty ChatView"
    );
}

#[test]
fn update_plan_text_updates_plan_block() {
    // When a Plan block exists, update_plan_text rewrites both its `raw` and
    // its `rendered` (markdown re-rendered from the new source).
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::Plan {
        rendered: crate::markdown::render("old plan"),
        raw: "old plan".to_string(),
    });
    v.update_plan_text("new plan text");
    match &v.blocks[0] {
        ChatBlock::Plan { raw, rendered } => {
            assert_eq!(raw, "new plan text", "Plan raw must be updated");
            assert_eq!(
                rendered,
                &crate::markdown::render("new plan text"),
                "Plan rendered must be re-rendered from the new text"
            );
        }
        other => panic!("expected Plan block, got {other:?}"),
    }
}

#[test]
fn update_plan_text_updates_assistant_when_no_plan() {
    // Without a Plan block, update_plan_text edits the last non-empty
    // Assistant block in place: raw is rewritten, rendered is re-rendered,
    // and `done` flips to true.
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::Assistant {
        raw: "original assistant text".to_string(),
        rendered: crate::markdown::render("original assistant text"),
        done: false,
    });
    v.update_plan_text("edited plan via assistant");
    match &v.blocks[0] {
        ChatBlock::Assistant {
            raw,
            rendered,
            done,
        } => {
            assert_eq!(
                raw, "edited plan via assistant",
                "Assistant raw must be updated"
            );
            assert_eq!(
                rendered,
                &crate::markdown::render("edited plan via assistant"),
                "Assistant rendered must be re-rendered"
            );
            assert!(
                *done,
                "done must flip to true after the plan edit is applied"
            );
        }
        other => panic!("expected Assistant block, got {other:?}"),
    }
}

/// Issue #1: the `[model]` chat marker must show the bare model id (no
/// `provider/` prefix), matching the status bar — both when the event carries
/// the full string (defensive strip) and the bare id (worker now emits bare).
#[test]
fn model_switch_marker_strips_provider_prefix() {
    let mut v = ChatView::default();

    // Full "provider/model" string -> rendered as bare id.
    v.apply(&SessionEvent::ModelSwitch("bigmodel/glm-5.2".into()));
    let text = block_text(&v);
    assert!(text.contains("[model]"), "marker prefix present");
    assert!(text.contains("glm-5.2"), "bare model id present");
    assert!(
        !text.contains('/'),
        "marker must not leak the provider slash: {text:?}"
    );

    // Bare id (what the worker now emits) -> unchanged, no slash.
    v.apply(&SessionEvent::ModelSwitch("glm-5.2".into()));
    let text2 = block_text(&v);
    assert!(text2.contains("glm-5.2"));
    assert!(!text2.contains('/'));
}

// ---------------------------------------------------------------------------
// P0: Tool-returned images render inline in the transcript (live path).
// When a ToolEnd carries `images`, each must produce a ChatBlock::Image.
// ---------------------------------------------------------------------------

/// Minimal valid 1×1 PNG as a data URI for tests.
fn tiny_png_data_uri() -> String {
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==".into()
}

#[test]
fn tool_end_with_images_renders_image_block() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t-img".into(),
        name: "view_image".into(),
        input: serde_json::json!({"path": "cat.png"}),
    });
    let uri = tiny_png_data_uri();
    v.apply(&SessionEvent::ToolEnd {
        id: "t-img".into(),
        name: "view_image".into(),
        output: "Loaded image: cat.png (0.1 KiB)".into(),
        is_error: false,
        images: vec![uri],
    });
    let images: Vec<_> = v
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Image { .. }))
        .collect();
    assert_eq!(
        images.len(),
        1,
        "expected exactly one Image block after ToolEnd with one image"
    );
    if let ChatBlock::Image { filename, .. } = images[0] {
        assert!(
            filename.contains("cat.png") || !filename.is_empty(),
            "image block should carry a display filename"
        );
    }
}

#[test]
fn tool_end_with_multiple_images_renders_all() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t-multi".into(),
        name: "view_image".into(),
        input: serde_json::json!({"path": "shot.png"}),
    });
    let uri = tiny_png_data_uri();
    v.apply(&SessionEvent::ToolEnd {
        id: "t-multi".into(),
        name: "view_image".into(),
        output: "done".into(),
        is_error: false,
        images: vec![uri.clone(), uri.clone(), uri],
    });
    let images: Vec<_> = v
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Image { .. }))
        .collect();
    assert_eq!(
        images.len(),
        3,
        "three images must produce three Image blocks"
    );
}

#[test]
fn tool_end_without_images_no_image_block() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t-text".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo hi"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "t-text".into(),
        name: "bash".into(),
        output: "hi".into(),
        is_error: false,
        images: Vec::new(),
    });
    let images: Vec<_> = v
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Image { .. }))
        .collect();
    assert!(
        images.is_empty(),
        "ToolEnd without images must not create Image blocks"
    );
}
