//! Step-group folding for runs of tool calls (`ChatBlock::StepGroup`).
//!
//! A group is a run of consecutive tool calls — any other block between two
//! calls (assistant text, image, marker) splits the run. Within a group a
//! step is one assistant round: a call merges into the trailing step while
//! that step holds no finished call, otherwise a NEW step opens (sequential
//! calls split, parallel calls share a step). Default render is ONE
//! clickable group row `▸ N steps`; clicking it lists the step rows,
//! clicking a step row reveals its thinking + `▸ N function calls`
//! aggregation row, clicking that lists the call rows, and clicking a call
//! row expands that call's output; Ctrl+L collapses every level again.

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
fn collapsed_by_default_renders_single_group_row() {
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
    // Group row + trailing blank: the whole ladder is one clickable row.
    assert_eq!(lines.len(), 2, "default render: group row + blank");
    assert!(
        lines[0].contains("\u{25b8} 2 steps"),
        "collapsed group row carries the step count: {:?}",
        lines[0]
    );
    assert!(
        !lines[0].contains("\u{276f}"),
        "closed group must use the closed glyph: {:?}",
        lines[0]
    );
    // Step rows, call headers and outputs are all folded away.
    assert!(!lines.iter().any(|l| l.contains("Step(")));
    assert!(!lines.iter().any(|l| l.contains("echo a")));
    assert!(!lines.iter().any(|l| l.contains("a-out")));

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
    assert_eq!(solo.len(), 2, "group row + blank");
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
fn toggling_the_ladder_reveals_each_level() {
    // Two sequential calls, no thinking. Default = group row + blank = 2;
    // group open = + 2 step rows = 4; step 1 open = + aggregation row = 5;
    // call list open = + call header = 6; call expanded = + output + blank
    // = 8.
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
    let list = flatten_text(&v);
    assert_eq!(list.len(), 2, "default = group row + blank");
    assert!(
        list[0].contains("\u{25b8} 2 steps"),
        "collapsed group row: {:?}",
        list[0]
    );

    // Open the group: both closed step rows appear.
    v.toggle_tool_call_at(0, 0);
    let open_group = flatten_text(&v);
    assert_eq!(open_group.len(), 4);
    assert!(
        open_group[0].contains("\u{276f} 2 steps"),
        "open group glyph: {:?}",
        open_group[0]
    );
    assert!(
        open_group[1].contains("\u{25b8} Step(1)") && open_group[2].contains("\u{25b8} Step(2)"),
        "both steps render collapsed rows: {open_group:?}"
    );

    // Open step 1: its aggregation row appears (no thinking → no Thinking
    // row).
    v.toggle_tool_call_at(0, 1);
    let open_step = flatten_text(&v);
    assert_eq!(open_step.len(), 5);
    assert!(
        open_step[1].contains("\u{276f} Step(1)"),
        "opened step row: {:?}",
        open_step[1]
    );
    assert!(
        open_step[2].contains("\u{25b8} 1 function call"),
        "aggregation row: {:?}",
        open_step[2]
    );

    // Open the call list: the call header row appears.
    v.toggle_tool_call_at(0, 2);
    let open_calls = flatten_text(&v);
    assert_eq!(open_calls.len(), 6);
    assert!(
        open_calls[2].contains("\u{276f} 1 function call"),
        "opened aggregation glyph: {:?}",
        open_calls[2]
    );
    assert!(
        open_calls[3].contains("echo a"),
        "call header: {:?}",
        open_calls[3]
    );

    // Expand that call: output + blank join the ladder.
    v.toggle_tool_call_at(0, 3);
    let expanded = flatten_text(&v);
    assert_eq!(expanded.len(), 8, "call output + blank join the ladder");
    assert!(
        expanded[4].contains("a-out"),
        "output visible: {:?}",
        expanded[4]
    );

    // Re-toggling the group collapses the whole ladder; the group row stays.
    v.toggle_tool_call_at(0, 0);
    let closed = flatten_text(&v);
    assert_eq!(closed.len(), 2, "group closed folds every level");
    assert!(
        closed[0].contains("\u{25b8} 2 steps"),
        "group row persists: {closed:?}"
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
fn collapse_all_collapsible_resets_groups_and_thinking() {
    let mut v = ChatView::default();
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
    // A pure-text round keeps a standalone Thinking block (a tool round
    // would absorb its thinking into the step).
    v.apply(&SessionEvent::ReasoningDelta("reason".into()));
    v.apply(&SessionEvent::TextDelta("spoken".into()));
    v.apply(&SessionEvent::Done);
    // Expand everything so they are observably NOT collapsed beforehand.
    for h in v.thinking_headers() {
        v.toggle_thinking_at(h.block_idx);
    }
    for b in v.blocks.iter_mut() {
        if let ChatBlock::StepGroup { open, steps, .. } = b {
            *open = true;
            steps[0].open = true;
            steps[0].calls_open = true; // call list open
            steps[0].calls[0].expanded = true; // call output expanded
        }
    }
    v.collapse_all_collapsible();
    for b in &v.blocks {
        match b {
            ChatBlock::Thinking { collapsed, .. } => {
                assert!(*collapsed, "thinking must be collapsed");
            }
            ChatBlock::StepGroup { open, steps, .. } => {
                assert!(
                    !*open
                        && steps.iter().all(|s| {
                            !s.open && !s.calls_open && s.calls.iter().all(|c| !c.expanded)
                        }),
                    "every ladder level must be collapsed after Ctrl+L"
                );
            }
            _ => {}
        }
    }
}

#[test]
fn step_row_line_index_lands_on_the_step_row() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("preamble\nsecond".into()));
    v.apply(&SessionEvent::Done);
    v.apply(&SessionEvent::ToolStart {
        id: "t".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    // Collapsed ladder: exactly one target — the group row (with its live
    // spinner hint, since the call is still running).
    let (group_idx, group_line) = {
        let headers = v.tool_call_headers();
        assert_eq!(headers.len(), 1, "expected exactly the group-row header");
        let line: String = v.flatten()[headers[0].header_line_idx]
            .spans
            .iter()
            .map(|s| s.content.clone())
            .collect();
        (headers[0].header_line_idx, line)
    };
    assert!(
        group_line.contains("1 step") && group_line.contains("running"),
        "header_line_idx must land on the group row; got: {group_line:?}"
    );

    // Open the group: the step row becomes the second target and must land
    // exactly one line below the group row.
    // `Done` leaves a marker block, so the group lands at index 2.
    v.toggle_tool_call_at(2, 0);
    let step_idx = v.tool_call_headers()[1].header_line_idx;
    let step_line: String = v.flatten()[step_idx]
        .spans
        .iter()
        .map(|s| s.content.clone())
        .collect();
    assert!(
        step_line.contains("Step(1)"),
        "header_line_idx must land on the step row; got: {step_line:?}"
    );
    assert_eq!(
        step_idx,
        group_idx + 1,
        "step row directly follows the group row"
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
