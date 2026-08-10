use super::super::*;

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
            ChatBlock::Tool {
                id, header, output, ..
            } => Some((id, header, output)),
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
            ChatBlock::Tool {
                id, header, output, ..
            } => Some((id, header, output)),
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
    crate::theme::set_theme(crate::theme::ThemeKind::Dark);
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
            ChatBlock::Tool {
                output, collapsed, ..
            } => Some((output, *collapsed)),
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
        matches!(
            v.blocks.last(),
            Some(ChatBlock::Tool {
                collapsed: true,
                ..
            })
        ),
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
        matches!(
            v.blocks.last(),
            Some(ChatBlock::Tool {
                collapsed: true,
                ..
            })
        ),
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
    assert!(
        block_text(&v).contains("hello"),
        "non-tool toggle must not corrupt state"
    );
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
fn tool_output_truncated_at_limit() {
    // Gap 2: even when expanded, a single ToolEnd event must not capture
    // an unbounded number of lines. The cap (TOOL_OUTPUT_LINES = 200)
    // bounds memory and per-refresh flatten_with cost.
    use crate::chat::TOOL_OUTPUT_LINES;
    let big: String = (0..5000)
        .map(|i| format!("line-{i}\n"))
        .collect::<String>()
        .trim_end()
        .to_string();
    assert!(
        big.lines().count() > TOOL_OUTPUT_LINES,
        "test setup: output must exceed the cap"
    );
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "big".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "cat huge_file.txt"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "big".into(),
        name: "bash".into(),
        output: big,
        is_error: false,
        images: Vec::new(),
    });
    let tool = v
        .blocks
        .iter()
        .find_map(|b| match b {
            ChatBlock::Tool { id, output, .. } if id == "big" => Some(output),
            _ => None,
        })
        .expect("tool block");
    assert_eq!(
        tool.len(),
        TOOL_OUTPUT_LINES,
        "tool output must be capped at TOOL_OUTPUT_LINES ({}), got {}",
        TOOL_OUTPUT_LINES,
        tool.len()
    );
    // Sanity: first line is the beginning of the output, not truncated from the front.
    let first: String = tool[0]
        .spans
        .iter()
        .map(|s| s.content.to_string())
        .collect();
    assert!(
        first.contains("line-0"),
        "first captured line must be the start of the output: {first}"
    );
}

#[test]
fn expanded_tool_header_prefix_arrow_flips_down() {
    // Regression: `flatten_with` rewrites the tool header's prefix arrow from
    // ▸ (U+25B8, points right) to ▾ (U+25BE, points down) when the block is
    // expanded, and reverts to ▸ when collapsed. The arrow is the visual cue
    // for whether the tool body is visible.
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

    let first_span_char =
        |lines: &[Line]| -> Option<char> { lines.first()?.spans.first()?.content.chars().next() };

    // Collapsed (default): header keeps ▸.
    assert_eq!(
        first_span_char(&v.flatten()),
        Some('\u{25b8}'),
        "collapsed tool header must start with ▸ (U+25B8)"
    );

    // Expand — arrow flips to ▾.
    v.toggle_tool_at(v.blocks.len() - 1);
    assert_eq!(
        first_span_char(&v.flatten()),
        Some('\u{25be}'),
        "expanded tool header must start with ▾ (U+25BE)"
    );

    // Collapse again — arrow reverts to ▸.
    v.toggle_tool_at(v.blocks.len() - 1);
    assert_eq!(
        first_span_char(&v.flatten()),
        Some('\u{25b8}'),
        "re-collapsed tool header must start with ▸ (U+25B8) again"
    );
}

#[test]
fn push_bash_tool_creates_expanded_tool_block() {
    // `push_bash_tool` opens a fresh tool block for a running bash command:
    // expanded (so the user sees output stream in), no output yet, and no
    // elapsed time recorded. Mirrors how `app_notepad` seeds a shell block.
    use crate::chat::{ChatBlock, ChatView};
    let mut v = ChatView::default();
    v.push_bash_tool("ls -la");

    match v.blocks.last() {
        Some(ChatBlock::Tool {
            id,
            header,
            output,
            collapsed,
            elapsed_ms,
            ..
        }) => {
            assert!(
                id.starts_with("bash-"),
                "id must start with 'bash-', got {id:?}"
            );
            let header_text: String = header.spans.iter().map(|s| s.content.clone()).collect();
            assert!(
                header_text.contains("ls -la"),
                "header must contain the command; got {header_text:?}"
            );
            assert!(
                output.is_empty(),
                "output must be empty before the command finishes"
            );
            assert!(
                !*collapsed,
                "a freshly-pushed tool block must be expanded (collapsed == false)"
            );
            assert_eq!(
                *elapsed_ms, None,
                "elapsed_ms must be None until finish_bash_tool is called"
            );
        }
        other => panic!("expected ChatBlock::Tool as last block, got {other:?}"),
    }
}

#[test]
fn finish_bash_tool_fills_output_and_collapses() {
    // After the command resolves, `finish_bash_tool` writes the captured
    // output lines, collapses the block, and stamps the elapsed time.
    use crate::chat::{ChatBlock, ChatView};
    let mut v = ChatView::default();
    v.push_bash_tool("echo hi");
    v.finish_bash_tool("hello\nworld");

    match v.blocks.last() {
        Some(ChatBlock::Tool {
            output,
            collapsed,
            elapsed_ms,
            ..
        }) => {
            assert!(
                !output.is_empty(),
                "output must contain lines after finish_bash_tool"
            );
            let joined: String = output
                .iter()
                .flat_map(|l| l.spans.iter())
                .map(|s| s.content.clone())
                .collect();
            assert!(
                joined.contains("hello") && joined.contains("world"),
                "output must preserve both lines; got {joined:?}"
            );
            assert!(
                *collapsed,
                "tool block must collapse once the command finishes"
            );
            assert!(
                elapsed_ms.is_some(),
                "elapsed_ms must be recorded after finish_bash_tool"
            );
        }
        other => panic!("expected ChatBlock::Tool as last block, got {other:?}"),
    }
}

#[test]
fn finish_bash_tool_aborted_message() {
    // When a command is aborted (e.g. user interrupt), the notepad layer
    // passes "(command aborted)" as the output. The block must surface that
    // text so the transcript explains why there is no real result.
    use crate::chat::{ChatBlock, ChatView};
    let mut v = ChatView::default();
    v.push_bash_tool("sleep 999");
    v.finish_bash_tool("(command aborted)");

    match v.blocks.last() {
        Some(ChatBlock::Tool { output, .. }) => {
            let joined: String = output
                .iter()
                .flat_map(|l| l.spans.iter())
                .map(|s| s.content.clone())
                .collect();
            assert!(
                joined.contains("aborted"),
                "aborted output must be visible in the block; got {joined:?}"
            );
        }
        other => panic!("expected ChatBlock::Tool as last block, got {other:?}"),
    }
}
