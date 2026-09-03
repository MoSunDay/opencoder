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
            thinking: Vec::new(),
            calls: vec![tool_call("t", 1)],
            open: false,
            calls_open: false,
        }],
        open: false,
    }
}

/// Verify the global invariant: the per-block line accounting in
/// `collect_headers` (which feeds mouse hit-rects via `*_headers()`) must sum
/// to exactly the number of lines `flatten_with` emits. If any single block
/// mis-counts, the total drifts and `expected != actual`.
fn assert_line_accounting_matches(view: &ChatView) {
    let actual = view.flatten_with(0, 0).len();

    // Reconstruct the expected total by running the SAME per-block accounting
    // that collect_headers uses. We do it independently here so a divergence
    // between collect_headers and flatten_with is caught regardless of which
    // side regresses. (collect_headers is private, so we mirror its math.)
    let mut expected = 0usize;
    for (block_idx, block) in view.blocks.iter().enumerate() {
        match block {
            ChatBlock::Marker(lines) => expected += lines.len(),
            ChatBlock::User { rendered } => expected += 1 + rendered.len(),
            ChatBlock::Assistant {
                raw,
                rendered,
                done,
            } => {
                if view.is_withheld_pub(block_idx) {
                    continue;
                }
                expected += 1;
                expected += if *done {
                    rendered.len()
                } else {
                    assistant_rows(raw).len()
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
            ChatBlock::StepGroup { steps, open } => {
                // Mirrors the StepGroup arm in collect_headers: group row,
                // then the three-level ladder while open, then one blank.
                expected += 1; // group row
                if *open {
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
                expected += 1; // trailing blank
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

/// Expose the private `is_withheld` for the test mirror. Implemented as an
/// extension trait so the production struct stays untouched.
trait WithheldPub {
    fn is_withheld_pub(&self, idx: usize) -> bool;
}
impl WithheldPub for ChatView {
    fn is_withheld_pub(&self, idx: usize) -> bool {
        self.hidden_assistant_idx == Some(idx) && self.subagents_running >= 1
    }
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
        line_text.contains("1 step"),
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
    assert!(line_text.contains("1 step"), "got {:?}", line_text);
}

#[test]
fn step_group_ladder_depth_alignment() {
    // The StepGroup line accounting must match flatten_with at EVERY ladder
    // depth — this is the invariant that keeps click hit-rects aligned.
    // 2 steps (1 call each, outputs 1 and 2 lines):
    //   group closed                       = 1 + 1
    //   group open, steps closed           = 1 + 2 + 1
    //   group open, steps open, calls shut = 1 + (1+1)*2 + 1
    //   fully open, call "a" expanded      = 1 + (1+1+1+3) + (1+1+1) + 1
    let mk = |group_open: bool, open_steps: bool, calls_open: bool, expand: bool| {
        let mut a = tool_call("a", 1);
        a.expanded = expand;
        view_with(vec![
            ChatBlock::StepGroup {
                steps: vec![
                    Step {
                        thinking: Vec::new(),
                        calls: vec![a],
                        open: open_steps,
                        calls_open,
                    },
                    Step {
                        thinking: Vec::new(),
                        calls: vec![tool_call("b", 2)],
                        open: open_steps,
                        calls_open,
                    },
                ],
                open: group_open,
            },
            marker_n(1),
        ])
    };

    // Collapsed group (the default): group row + trailing blank only.
    let v = mk(false, false, false, false);
    assert_line_accounting_matches(&v);
    assert_eq!(v.flatten().len(), 2 + 1);

    // Group open, steps closed: group row + 2 step rows + blank.
    let v = mk(true, false, false, false);
    assert_line_accounting_matches(&v);
    assert_eq!(v.flatten().len(), 4 + 1);

    // Steps open, call lists shut: + aggregation row per step.
    let v = mk(true, true, false, false);
    assert_line_accounting_matches(&v);
    assert_eq!(v.flatten().len(), 6 + 1);

    // Fully open; only call "a" is expanded so it also renders its 1 output
    // line + separator blank:
    // group(1) + S1(1)+agg(1)+a hdr(1)+a out(1)+sep(1)
    //          + S2(1)+agg(1)+b hdr(1) + trailing blank(1) + marker(1) = 11.
    let v = mk(true, true, true, true);
    assert_line_accounting_matches(&v);
    assert_eq!(v.flatten().len(), 11);
}

#[test]
fn step_group_with_expanded_call_keeps_alignment() {
    // Per-call expansion in an open call list: only call "a" shows its
    // output. The recorded hit rows must point at the rendered group/step/
    // aggregation/call rows, and expanding a call must shift the rows after
    // it by exactly its output + separator.
    let mut a = tool_call("a", 1);
    a.expanded = true;
    let v = view_with(vec![
        ChatBlock::StepGroup {
            steps: vec![
                Step {
                    thinking: Vec::new(),
                    calls: vec![a],
                    open: true,
                    calls_open: true,
                },
                Step {
                    thinking: Vec::new(),
                    calls: vec![tool_call("b", 2)],
                    open: true,
                    calls_open: true,
                },
            ],
            open: true,
        },
        marker_n(1),
    ]);
    assert_line_accounting_matches(&v);
    // group(1) + S1 row(1) + agg(1) + a hdr(1) + a out(1) + sep(1)
    // + S2 row(1) + agg(1) + b hdr(1) + trailing blank(1) + marker(1) = 11.
    assert_eq!(v.flatten().len(), 11);

    let call_headers = v.tool_call_headers();
    assert_eq!(
        call_headers.len(),
        7,
        "group row + 2 step rows + 2 aggregation rows + 2 call header rows"
    );
    assert_eq!(call_headers[0].call_idx, 0, "group row");
    assert_eq!(call_headers[0].header_line_idx, 0);
    assert_eq!(call_headers[1].call_idx, 1, "step 1 row");
    assert_eq!(call_headers[1].header_line_idx, 1);
    assert_eq!(call_headers[2].call_idx, 2, "step 1 aggregation row");
    assert_eq!(call_headers[2].header_line_idx, 2);
    assert_eq!(call_headers[3].call_idx, 3, "call a's header row");
    assert_eq!(call_headers[3].header_line_idx, 3);
    assert_eq!(
        call_headers[4].header_line_idx, 6,
        "step 2 sits after the expanded output + separator"
    );
    assert_eq!(call_headers[5].header_line_idx, 7, "step 2 aggregation row");
    assert_eq!(call_headers[6].call_idx, 6, "call b's header row");
    assert_eq!(call_headers[6].header_line_idx, 8);
    // Each recorded row must be the group row (`{❯|▸} N step(s)`), a step
    // row (`{❯|▸} Step(N)`), an aggregation row (`{❯|▸} N function call(s)`)
    // or a call header line — all share the `{❯|▸} ` gutter prefix.
    let flat = v.flatten();
    for h in &call_headers {
        let text: String = flat[h.header_line_idx]
            .spans
            .iter()
            .map(|s| s.content.clone())
            .collect();
        let t = text.trim_start();
        assert!(
            t.starts_with("\u{25b8} ") || t.starts_with("\u{276f} "),
            "row {} is not a ladder row: {text:?}",
            h.header_line_idx
        );
    }
}

#[test]
fn step_group_running_call_keeps_alignment() {
    // An unfinished call (elapsed_ms == None) adds a spinner SPAN to the
    // group row — never an extra line. Accounting must stay identical.
    let v = view_with(vec![
        ChatBlock::StepGroup {
            steps: vec![Step {
                thinking: Vec::new(),
                calls: vec![ToolCall {
                    id: "r".into(),
                    header: Line::from(Span::raw("\u{25b8} bash")),
                    output: Vec::new(),
                    started_at_ms: Some(0),
                    elapsed_ms: None,
                    expanded: false,
                }],
                open: false,
                calls_open: false,
            }],
            open: false,
        },
        marker_n(1),
    ]);
    assert_line_accounting_matches(&v);
    // group row + trailing blank + marker_n.
    assert_eq!(v.flatten().len(), 2 + 1);
    // The spinner hint rides on the group row itself.
    let row: String = v.flatten()[0]
        .spans
        .iter()
        .map(|s| s.content.clone())
        .collect();
    assert!(
        row.contains("running"),
        "group row should hint running: {row:?}"
    );
}
