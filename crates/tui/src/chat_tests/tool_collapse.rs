//! Step-group folding for runs of tool calls (`ChatBlock::StepGroup`).
//!
//! A group is a run of consecutive tool calls — any other block between two
//! calls (assistant text, image, marker) splits the run. Within a group a
//! step is one assistant round: a call merges into the trailing step while
//! that step holds no finished call, otherwise a NEW step opens (sequential
//! calls split, parallel calls share a step). Default render is a single
//! collapsed line carrying the step count; clicking the group row toggles
//! the whole group; Ctrl+L collapses every group/step/call again.

use super::super::*;

/// Collect `(group_idx, steps)` for every StepGroup in the view.
fn groups(v: &ChatView) -> Vec<(usize, &Vec<Step>)> {
    v.blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            ChatBlock::StepGroup { steps, .. } => Some((i, steps)),
            _ => None,
        })
        .collect()
}

/// Flatten one group's steps into its calls in order.
fn group_calls(v: &ChatView) -> Vec<Vec<&ToolCall>> {
    groups(v)
        .iter()
        .map(|(_, steps)| steps.iter().flat_map(|s| s.calls.iter()).collect())
        .collect()
}

fn flatten_text(v: &ChatView) -> Vec<String> {
    v.flatten()
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn parallel_tool_calls_form_one_group_and_route_by_id() {
    // Regression: when two tools start before either ends (parallel bash
    // calls), they join ONE group (and one step, neither being finished at
    // the second start) and each ToolEnd must append output to its own call
    // by id — not to the last-pushed call.
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

    let grps = group_calls(&v);
    assert_eq!(grps.len(), 1, "concurrent calls must form one group");
    assert_eq!(grps[0].len(), 2, "the step must hold both calls");
    // Calls keep start order regardless of end order.
    assert_eq!(grps[0][0].id, "a");
    assert_eq!(grps[0][1].id, "b");
    let text = |c: &ToolCall| -> String {
        c.header
            .spans
            .iter()
            .chain(c.output.iter().flat_map(|l| l.spans.iter()))
            .map(|s| s.content.clone())
            .collect()
    };
    let text_a = text(grps[0][0]);
    let text_b = text(grps[0][1]);
    assert!(text_a.contains("echo A"), "call A header: {text_a}");
    assert!(text_a.contains("A-out"), "call A output: {text_a}");
    assert!(!text_a.contains("B-out"), "call A contaminated: {text_a}");
    assert!(text_b.contains("echo B"), "call B header: {text_b}");
    assert!(text_b.contains("B-out"), "call B output: {text_b}");
    assert!(!text_b.contains("A-out"), "call B contaminated: {text_b}");
    // Finished calls record elapsed time.
    assert!(grps[0].iter().all(|c| c.elapsed_ms.is_some()));
}

#[test]
fn sequential_calls_split_into_steps() {
    // One round with two SEQUENTIAL calls: once the first call finished, the
    // next start opens a NEW step of the same group.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "a".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo A"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "a".into(),
        name: "bash".into(),
        output: "A-out".into(),
        is_error: false,
        images: Vec::new(),
    });
    v.apply(&SessionEvent::ToolStart {
        id: "b".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo B"}),
    });
    let grps = groups(&v);
    assert_eq!(grps.len(), 1, "sequential calls stay in one group");
    assert_eq!(grps[0].1.len(), 2, "finished call forces a new step");
    assert_eq!(grps[0].1[0].calls[0].id, "a");
    assert_eq!(grps[0].1[1].calls[0].id, "b");
}

#[test]
fn collapsed_by_default_renders_single_count_line() {
    let mut v = ChatView::default();
    for id in ["a", "b"] {
        v.apply(&SessionEvent::ToolStart {
            id: id.into(),
            name: "bash".into(),
            input: serde_json::json!({"command": format!("echo {id}")}),
        });
        v.apply(&SessionEvent::ToolEnd {
            id: id.into(),
            name: "bash".into(),
            output: format!("{id}-out\nsecond line"),
            is_error: false,
            images: Vec::new(),
        });
    }
    let lines = flatten_text(&v);
    assert_eq!(lines.len(), 1, "collapsed group renders exactly one line");
    assert!(
        lines[0].contains("2 steps"),
        "collapsed line carries the step count: {:?}",
        lines[0]
    );
    // Collapsed hides all call headers and outputs.
    assert!(!lines[0].contains("echo A"));
    assert!(!lines[0].contains("A-out"));

    // Singular grammar for a single-step group (two parallel calls = one
    // step).
    let mut v1 = ChatView::default();
    v1.apply(&SessionEvent::ToolStart {
        id: "solo".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "true"}),
    });
    v1.apply(&SessionEvent::ToolEnd {
        id: "solo".into(),
        name: "bash".into(),
        output: "ok".into(),
        is_error: false,
        images: Vec::new(),
    });
    let solo = flatten_text(&v1);
    assert_eq!(solo.len(), 1);
    assert!(
        solo[0].contains("1 step") && !solo[0].contains("steps"),
        "single step uses singular: {:?}",
        solo[0]
    );
}

#[test]
fn running_hint_in_group_line_while_call_unfinished() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "slow".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "sleep 5"}),
    });
    let line = &flatten_text(&v)[0];
    assert!(
        line.contains("running"),
        "unfinished call must show the running hint: {line:?}"
    );
    v.apply(&SessionEvent::ToolEnd {
        id: "slow".into(),
        name: "bash".into(),
        output: "done".into(),
        is_error: false,
        images: Vec::new(),
    });
    let line = &flatten_text(&v)[0];
    assert!(
        !line.contains("running"),
        "finished call must drop the running hint: {line:?}"
    );
}

#[test]
fn toggling_steps_reveals_the_ladder() {
    // Two sequential calls, no thinking. Collapsed = 1 line; group open =
    // group row + 2 step rows + trailing blank = 4; a step opened adds its
    // call header = 5; the call expanded adds output + blank = 7.
    let mut v = ChatView::default();
    for id in ["a", "b"] {
        v.apply(&SessionEvent::ToolStart {
            id: id.into(),
            name: "bash".into(),
            input: serde_json::json!({"command": format!("echo {id}")}),
        });
        v.apply(&SessionEvent::ToolEnd {
            id: id.into(),
            name: "bash".into(),
            output: format!("{id}-out"),
            is_error: false,
            images: Vec::new(),
        });
    }
    assert_eq!(v.flatten().len(), 1, "collapsed");

    v.toggle_step_group_at(0);
    let list = flatten_text(&v);
    assert_eq!(list.len(), 4, "open = group row + 2 step rows + blank");
    assert!(
        list[0].contains("\u{25be}"),
        "expanded marker: {:?}",
        list[0]
    );
    assert!(
        list[1].contains("\u{25b8} Step(1)") && list[2].contains("\u{25b8} Step(2)"),
        "both steps render collapsed rows: {list:?}"
    );

    // Open step 1: its row flips to the open glyph and its call header row
    // appears (no thinking -> no Say row).
    v.toggle_tool_call_at(0, 0);
    let open_step = flatten_text(&v);
    assert_eq!(open_step.len(), 5);
    assert!(
        open_step[1].contains("\u{276f} Step(1)"),
        "opened step row: {:?}",
        open_step[1]
    );
    assert!(
        open_step[2].contains("echo a"),
        "call header: {:?}",
        open_step[2]
    );

    // Expand that call (flat target 1 = step 1's call): output + blank.
    v.toggle_tool_call_at(0, 1);
    let expanded = flatten_text(&v);
    assert_eq!(expanded.len(), 7, "call output + blank join the ladder");
    assert!(
        expanded[3].contains("a-out"),
        "output visible: {:?}",
        expanded[3]
    );

    // Re-toggling the step collapses it (and its rows) again.
    v.toggle_tool_call_at(0, 0);
    assert_eq!(v.flatten().len(), 4, "step closed removes its rows");

    // Closing the group returns to the single count line.
    v.toggle_step_group_at(0);
    assert_eq!(v.flatten().len(), 1, "group closed -> one line");
    assert!(
        matches!(
            v.blocks.last(),
            Some(ChatBlock::StepGroup { open: false, .. })
        ),
        "group must be closed after the round trip"
    );
}

#[test]
fn text_between_calls_splits_groups_and_backfills_older_group() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "one"}),
    });
    v.apply(&SessionEvent::TextDelta("thinking out loud".into()));
    v.apply(&SessionEvent::Done);
    v.apply(&SessionEvent::ToolStart {
        id: "t2".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "two"}),
    });
    // The assistant block split the run: two groups.
    let grps = group_calls(&v);
    assert_eq!(grps.len(), 2, "text between calls splits the run");
    assert_eq!(grps[0].len(), 1);
    assert_eq!(grps[1].len(), 1);

    // Ending the older group's call after the newer group exists must still
    // route into the older group by id.
    v.apply(&SessionEvent::ToolEnd {
        id: "t1".into(),
        name: "bash".into(),
        output: "first-out".into(),
        is_error: false,
        images: Vec::new(),
    });
    assert!(
        group_calls(&v)[0][0]
            .output
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains("first-out"))),
        "output must land in the older group's call"
    );
    // Appending a call while an Assistant block is trailing starts a NEW
    // group (the assistant text splits the run).
    assert_eq!(groups(&v).len(), 2);
}

#[test]
fn orphan_tool_end_creates_synthetic_group() {
    // A ToolEnd with no preceding ToolStart (e.g. a lost event) must not
    // panic; it creates a synthetic finished single-call group carrying the
    // id and "(output)" header.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolEnd {
        id: "orphan".into(),
        name: "bash".into(),
        output: "loose output".into(),
        is_error: false,
        images: Vec::new(),
    });
    let grps = group_calls(&v);
    assert_eq!(grps.len(), 1, "orphan ToolEnd creates one group");
    assert_eq!(grps[0].len(), 1);
    let call = grps[0][0];
    assert_eq!(call.id, "orphan");
    let header: String = call
        .header
        .spans
        .iter()
        .map(|s| s.content.clone())
        .collect();
    assert!(header.contains("(output)"), "synthetic header: {header}");
    let out: String = call
        .output
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect();
    assert!(out.contains("loose output"), "output appended: {out}");
    assert!(
        call.elapsed_ms.is_some(),
        "synthetic call is finished (no running hint)"
    );
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
    let call = group_calls(&v)[0][0];
    let span = &call
        .output
        .first()
        .expect("output line")
        .spans
        .first()
        .expect("output span");
    assert!(
        span.style.fg == Some(ratatui::style::Color::Red),
        "error output must be err-colored, got {:?}",
        span.style
    );
}

#[test]
fn toggle_step_group_at_is_noop_for_non_tool_blocks() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hello".into()));
    v.apply(&SessionEvent::Done);
    // Index 0 is an Assistant block, not a StepGroup — cycling must be a
    // no-op.
    v.toggle_step_group_at(0);
    assert!(
        block_text(&v).contains("hello"),
        "non-tool cycle must not corrupt state"
    );
}

#[test]
fn collapse_all_collapsible_resets_groups_and_thinking() {
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
    // Expand everything so they are observably NOT collapsed beforehand.
    for h in v.thinking_headers() {
        v.toggle_thinking_at(h.block_idx);
    }
    for h in v.tool_headers() {
        v.toggle_step_group_at(h.block_idx);
        v.toggle_tool_call_at(h.block_idx, 0); // step open
        v.toggle_tool_call_at(h.block_idx, 1); // call output expanded
    }
    v.collapse_all_collapsible();
    for b in &v.blocks {
        match b {
            ChatBlock::Thinking { collapsed, .. } => {
                assert!(*collapsed, "thinking must be collapsed");
            }
            ChatBlock::StepGroup { steps, open } => {
                assert!(!open, "group must be closed after Ctrl+L");
                assert!(
                    steps
                        .iter()
                        .all(|s| !s.open && s.calls.iter().all(|c| !c.expanded)),
                    "steps and call outputs must be collapsed after Ctrl+L"
                );
            }
            _ => {}
        }
    }
}

#[test]
fn tool_headers_line_index_lands_on_group_line() {
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
        header_line.contains("1 step"),
        "header_line_idx must land on the group line; got: {header_line:?}"
    );
}

#[test]
fn tool_header_keeps_full_command_text() {
    // The header row of a tool call can exceed the terminal width, hiding
    // the real command behind an ellipsis. summarize() must return the full
    // command text so the body layer can wrap it to the terminal width.
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
            ChatBlock::StepGroup { steps, .. } => Some(steps[0].calls[0].header.clone()),
            _ => None,
        })
        .expect("step group");
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
    // Even with a fully open ladder, a single ToolEnd event must not capture
    // an unbounded number of lines. The cap (TOOL_OUTPUT_LINES = 200) bounds
    // memory and per-refresh flatten_with cost.
    use crate::chat::TOOL_OUTPUT_LINES;
    let big: String = (0..500).map(|i| format!("line {i}\n")).collect();
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "big".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "yes"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "big".into(),
        name: "bash".into(),
        output: big,
        is_error: false,
        images: Vec::new(),
    });
    let call = group_calls(&v)[0][0];
    assert_eq!(
        call.output.len(),
        TOOL_OUTPUT_LINES,
        "captured output must be capped at TOOL_OUTPUT_LINES"
    );
}
