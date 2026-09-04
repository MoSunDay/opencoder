use super::super::*;
use ratatui::text::{Line, Span};

/// Build an empty-image block (the A2 case): `rendered` empty → flatten_with
/// emits a `"(unable to render)"` placeholder.
fn image_empty(name: &str) -> ChatBlock {
    ChatBlock::Image {
        filename: name.into(),
        rendered: Vec::new(),
    }
}

/// Build a non-empty image block with N rendered lines.
fn image_n(name: &str, n: usize) -> ChatBlock {
    ChatBlock::Image {
        filename: name.into(),
        rendered: (0..n).map(|_| Line::from(Span::raw("#"))).collect(),
    }
}

/// Build a streaming (`done: false`) assistant block.
fn assistant_streaming(raw: &str) -> ChatBlock {
    ChatBlock::Assistant {
        raw: raw.into(),
        rendered: Vec::new(),
        done: false,
    }
}

/// Build a finalized (`done: true`) assistant block with the given rendered lines.
fn assistant_done(n: usize) -> ChatBlock {
    ChatBlock::Assistant {
        raw: String::new(),
        rendered: (0..n).map(|_| Line::from(Span::raw("ok"))).collect(),
        done: true,
    }
}

/// Build a marker block with `n` lines.
fn marker_n(n: usize) -> ChatBlock {
    ChatBlock::Marker((0..n).map(|_| Line::from(Span::raw(""))).collect())
}

/// Build one tool call with `out_lines` output lines.
fn tool_call(id: &str, out_lines: usize) -> ToolCall {
    ToolCall {
        id: id.into(),
        header: Line::from(Span::raw(format!("\u{25b8} {id}"))),
        output: (0..out_lines)
            .map(|_| Line::from(Span::raw("out")))
            .collect(),
        started_at_ms: Some(0),
        elapsed_ms: Some(0),
        expanded: false,
    }
}

/// Build a single-call step group in its default (collapsed) state: the
/// group row + trailing blank = 2 lines.
fn step_group_closed() -> ChatBlock {
    ChatBlock::StepGroup {
        steps: vec![Step {
            thinking_raw: String::new(),
            thinking: Vec::new(),
            thinking_dirty: false,
            calls: vec![tool_call("t", 1)],
            open: false,
            calls_open: false,
            sealed: false,
        }],
        open: false,
        progress_active: false,
    }
}

/// Verify the global invariant: the per-block line accounting in
/// `collect_headers` (which feeds mouse hit-rects via `*_headers()`) must sum
/// to exactly the number of lines `flatten_with` emits. If any single block
/// mis-counts, the total drifts and `expected != actual`.
pub(super) fn assert_line_accounting_matches(view: &ChatView) {
    let actual = view.flatten_with(0, 0).len();

    // Reconstruct the expected total by running the SAME per-block accounting
    // that collect_headers uses. We do it independently here so a divergence
    // between collect_headers and flatten_with is caught regardless of which
    // side regresses. (collect_headers is private, so we mirror its math.)
    let mut expected = 0usize;
    for (bi, block) in view.blocks.iter().enumerate() {
        // ADJACENT-pair merge (see flatten_with): a StepGroup whose NEXT
        // block is the turn's Say renders one merged header row and the Say
        // renders body only — both sides of the pair lose one line.
        let say_merged_after_group =
            matches!(view.blocks.get(bi + 1), Some(ChatBlock::Assistant { .. }));
        let say_merged_into_group = bi
            .checked_sub(1)
            .and_then(|i| view.blocks.get(i))
            .is_some_and(|b| matches!(b, ChatBlock::StepGroup { .. }));
        match block {
            ChatBlock::Marker(lines) => expected += lines.len(),
            ChatBlock::User { rendered } => expected += 1 + rendered.len(),
            ChatBlock::Assistant {
                raw,
                rendered,
                done,
            } => {
                if !say_merged_into_group {
                    expected += 1;
                }
                let total = if *done {
                    rendered.len()
                } else {
                    assistant_rows(raw).len()
                };
                // 合并对正文行数：跳过与 preview 重复的首个非空行（单行
                // Say / 空正文整块隐藏）—— 与 flatten_with 同口径。
                expected += if say_merged_into_group {
                    super::super::step_render::merged_say_body_decision(raw, rendered, *done)
                        .visible_len(total)
                } else {
                    total
                };
            }
            ChatBlock::Thinking {
                text, collapsed, ..
            } => {
                expected += 1;
                if !collapsed {
                    expected += text.lines().count();
                }
            }
            ChatBlock::Compaction {
                text, collapsed, ..
            } => {
                expected += 1;
                if !collapsed {
                    expected += text.lines().count();
                }
            }
            ChatBlock::StepGroup { steps, open, .. } => {
                // Mirrors the StepGroup arm in collect_headers: group row
                // (or merged Say header when the next block is the Say) plus
                // its separator blank, then the three-level ladder while
                // open, then one trailing blank — the LADDER blank is
                // skipped for a CLOSED merged pair (the header's separator
                // blank already terminates it).
                expected += 1; // group row
                if say_merged_after_group {
                    // 合并头部行之后的空行（与 flatten_step_group 同步）。
                    expected += 1;
                }
                if !*open && say_merged_after_group {
                    // merged closed pair: no trailing blank
                } else if *open {
                    for s in steps {
                        expected += 1; // step row
                        if s.open {
                            if !s.thinking.is_empty() {
                                expected += 1 + s.thinking.len();
                            }
                            if !s.calls.is_empty() {
                                expected += 1; // calls aggregation row
                                if s.calls_open {
                                    for c in &s.calls {
                                        expected +=
                                            1 + if c.expanded { 1 + c.output.len() } else { 0 };
                                    }
                                }
                            }
                        }
                    }
                }
                // Exactly one blank after the group: a final expanded
                // call's separator blank (counted per call above) doubles as
                // the trailing blank.
                let ends_on_expanded_call = *open
                    && steps.last().is_some_and(|s| {
                        s.open && s.calls_open && s.calls.last().is_some_and(|c| c.expanded)
                    });
                if !ends_on_expanded_call && !(!*open && say_merged_after_group) {
                    expected += 1; // trailing blank
                }
            }
            ChatBlock::Image { rendered, .. } => {
                expected +=
                    1 + if rendered.is_empty() {
                        1
                    } else {
                        rendered.len()
                    } + 1;
            }
            ChatBlock::Subagent { .. } => {
                expected += 1;
            }
            ChatBlock::Plan { rendered, .. } => {
                expected += 1 + rendered.len() + 1;
            }
        }
    }
    assert_eq!(
        actual, expected,
        "flatten_with emitted {actual} lines but collect_headers accounts {expected}"
    );
}

fn view_with(blocks: Vec<ChatBlock>) -> ChatView {
    ChatView {
        blocks,
        ..Default::default()
    }
}

#[test]
fn image_empty_drifts_alignment_a2() {
    // Bug A2: an empty-rendered Image emits a "(unable to render)" placeholder
    // (3 lines total). A non-empty-following block must still align.
    let v = view_with(vec![image_empty("a.png"), marker_n(2)]);
    assert_line_accounting_matches(&v);

    // The marker should start at line index 3 (header + placeholder + blank).
    let flat = v.flatten();
    assert_eq!(flat.len(), 5);
    // header[0] = "[image: a.png]", [1] = placeholder, [2] = blank, [3..] marker
    let join: Vec<String> = flat
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    assert!(join[0].contains("a.png"));
    assert!(join[1].contains("unable to render"));
}

#[test]
fn image_nonempty_alignment() {
    let v = view_with(vec![image_n("b.png", 2), marker_n(1)]);
    assert_line_accounting_matches(&v);
    assert_eq!(v.flatten().len(), 1 + 2 + 1 + 1);
}

#[test]
fn assistant_streaming_trailing_newline_a3() {
    // Bug A3: `raw` ends with `\n`; collect_headers must NOT count the
    // trailing empty split element as an extra body line.
    let v = view_with(vec![assistant_streaming("only\n"), marker_n(1)]);
    assert_line_accounting_matches(&v);
    // 1 (say header) + 1 (body "only") + 1 (marker) = 3
    assert_eq!(v.flatten().len(), 3);
}

#[test]
fn assistant_streaming_no_trailing_newline() {
    let v = view_with(vec![assistant_streaming("a\nb"), marker_n(1)]);
    assert_line_accounting_matches(&v);
    // 1 header + 2 body + 1 marker = 4
    assert_eq!(v.flatten().len(), 4);
}

#[test]
fn assistant_done_alignment() {
    let v = view_with(vec![assistant_done(3), marker_n(1)]);
    assert_line_accounting_matches(&v);
    // 1 header + 3 rendered + 1 marker = 5
    assert_eq!(v.flatten().len(), 5);
}

#[test]
fn marker_only_alignment() {
    let v = view_with(vec![marker_n(4)]);
    assert_line_accounting_matches(&v);
    assert_eq!(v.flatten().len(), 4);
}

#[test]
fn mixed_sequence_alignment() {
    // All six required cases composed, with a trailing step group so we can
    // also verify header line indices point at the right rendered line.
    let v = view_with(vec![
        image_empty("x.png"),        // 3 lines
        image_n("y.png", 2),         // 4 lines
        assistant_streaming("p\n"),  // 2 lines
        assistant_streaming("q\nr"), // 3 lines
        assistant_done(2),           // 3 lines
        marker_n(1),                 // 1 line
        step_group_closed(),         // 2 lines (group row + blank)
    ]);
    assert_line_accounting_matches(&v);

    // Verify the group-row header_line_idx points at the actual group row.
    let flat = v.flatten();
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 1, "expected exactly the group row");
    let idx = headers[0].header_line_idx;
    assert!(
        idx < flat.len(),
        "header_line_idx {idx} out of range (flat len {})",
        flat.len()
    );
    let line_text: String = flat[idx].spans.iter().map(|s| s.content.clone()).collect();
    assert!(
        line_text.contains("1 Step"),
        "header_line_idx {idx} points at {:?}, expected the group row",
        line_text
    );
    // Expected group-row position = 3 + 4 + 2 + 3 + 3 + 1 (marker) = 16.
    assert_eq!(idx, 16, "group row should land at line 16");
}

#[test]
fn empty_image_followed_by_tool_alignment() {
    // The original A2 failure mode: empty Image then a Tool block. Before the
    // fix collect_headers counted 2 for the Image but flatten_with emitted 3,
    // so the tool hit-rect landed one line too early.
    let v = view_with(vec![image_empty("z.png"), step_group_closed()]);
    assert_line_accounting_matches(&v);

    let flat = v.flatten();
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 1);
    let idx = headers[0].header_line_idx;
    // 0=image header, 1=placeholder, 2=blank, 3=group row (collapsed group).
    assert_eq!(idx, 3);
    let line_text: String = flat[idx].spans.iter().map(|s| s.content.clone()).collect();
    assert!(line_text.contains("1 Step"), "got {:?}", line_text);
}

#[path = "line_accounting/step_ladder.rs"]
mod step_ladder;
