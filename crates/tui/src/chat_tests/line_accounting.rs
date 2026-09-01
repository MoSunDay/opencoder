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

/// Build a collapsed tool block (header + output). Collapsed tool emits 1 line.
fn tool_collapsed() -> ChatBlock {
    ChatBlock::Tool {
        id: "t".into(),
        header: Line::from(Span::raw("bash")),
        output: vec![Line::from(Span::raw("hi"))],
        collapsed: true,
        started_at_ms: 0,
        elapsed_ms: None,
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
            ChatBlock::Tool {
                header,
                output,
                collapsed,
                ..
            } => {
                if *collapsed {
                    expected += 1;
                } else {
                    expected += 1 + output.len() + 1;
                    let _ = header;
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
            ChatBlock::Sidecar { .. } => {
                expected += 1; // header-only row, mirrors collect_headers
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
    // All six required cases composed, with a trailing tool header so we can
    // also verify header line indices point at the right rendered line.
    let v = view_with(vec![
        image_empty("x.png"),        // 3 lines
        image_n("y.png", 2),         // 4 lines
        assistant_streaming("p\n"),  // 2 lines
        assistant_streaming("q\nr"), // 3 lines
        assistant_done(2),           // 3 lines
        marker_n(1),                 // 1 line
        tool_collapsed(),            // 1 line
    ]);
    assert_line_accounting_matches(&v);

    // Verify the tool header_line_idx points at the actual tool header line.
    let flat = v.flatten();
    let tool_headers = v.tool_headers();
    assert_eq!(tool_headers.len(), 1, "expected exactly one tool header");
    let idx = tool_headers[0].header_line_idx;
    assert!(
        idx < flat.len(),
        "header_line_idx {idx} out of range (flat len {})",
        flat.len()
    );
    // The line at that index must carry the tool header content "bash".
    let line_text: String = flat[idx].spans.iter().map(|s| s.content.clone()).collect();
    assert!(
        line_text.contains("bash"),
        "header_line_idx {idx} points at {:?}, expected the tool header",
        line_text
    );
    // Expected tool header position = 3 + 4 + 2 + 3 + 3 + 1 = 16.
    assert_eq!(idx, 16, "tool header should land at line 16");
}

#[test]
fn empty_image_followed_by_tool_alignment() {
    // The original A2 failure mode: empty Image then a Tool block. Before the
    // fix collect_headers counted 2 for the Image but flatten_with emitted 3,
    // so the tool hit-rect landed one line too early.
    let v = view_with(vec![image_empty("z.png"), tool_collapsed()]);
    assert_line_accounting_matches(&v);

    let flat = v.flatten();
    let tool_headers = v.tool_headers();
    assert_eq!(tool_headers.len(), 1);
    let idx = tool_headers[0].header_line_idx;
    // After fix: header at index 3 (0=image header,1=placeholder,2=blank,3=tool).
    assert_eq!(idx, 3);
    let line_text: String = flat[idx].spans.iter().map(|s| s.content.clone()).collect();
    assert!(line_text.contains("bash"), "got {:?}", line_text);
}
