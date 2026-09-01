use super::*;
use crate::composer;

mod agent_switch;
mod compaction_state;
mod image_render;
mod line_accounting;
mod plan_card;
mod sidecar_fold;
mod steer_echo;
mod subagent;
mod terminal_safety;
mod thinking_state;
mod timer;
mod tok_cost;
mod tool_collapse;
mod user_block;

#[test]
fn llm_round_lifecycle_is_display_only_and_resets_at_boundary() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 1000,
    });
    v.apply(&SessionEvent::TextDelta("working".into()));
    assert_eq!(v.llm_round_started_at_ms, Some(1000));
    let before = block_text(&v);
    assert!(before.contains("working"));
    assert!(!before.contains("turn cost"));

    v.apply(&SessionEvent::LlmRoundEnd);
    assert_eq!(v.llm_round_started_at_ms, None);
    assert!(
        v.frozen_round_ms.is_some(),
        "LlmRoundEnd must freeze the round cost"
    );
    assert_eq!(block_text(&v), before, "timer data is not message text");
}

#[test]
fn terminal_event_clears_an_unfinished_round() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 1000,
    });
    v.apply(&SessionEvent::Done);
    assert_eq!(v.llm_round_started_at_ms, None);
}

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
fn ctx_counts_queue_consumed_prompt() {
    let mut v = ChatView::default();
    let before = v.context_used;
    v.apply(&SessionEvent::QueueConsumed {
        seq: 5,
        text: "a queued user message".into(),
    });
    // Queued prompts are real user messages the model sees in context;
    // they must be counted so the ctx% meter matches the compaction budget.
    assert_eq!(
        v.context_used,
        before + estimate("a queued user message") as u64,
        "QueueConsumed must add its prompt text to context_used"
    );
}

#[test]
fn ctx_counts_steer_consumed_prompt() {
    let mut v = ChatView::default();
    let before = v.context_used;
    v.apply(&SessionEvent::SteerConsumed {
        seq: 7,
        text: "a steered redirection".into(),
    });
    // Steered prompts are real user messages the model sees in context;
    // they must be counted so the ctx% meter matches the compaction budget.
    assert_eq!(
        v.context_used,
        before + estimate("a steered redirection") as u64,
        "SteerConsumed must add its prompt text to context_used"
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
