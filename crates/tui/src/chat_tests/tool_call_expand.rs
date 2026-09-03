//! Per-row drill-down inside a `StepGroup` (Phase 2 of clickable function
//! calls): clicking a rendered row toggles ONLY that target — a step row
//! flips that step open/closed, a call header row toggles ONLY that call's
//! output. Rows are enumerated by the group's VISIBLE targets (step rows,
//! plus each open step's call rows, in render order).

use super::super::*;

/// Two-step group built through the real event path (sequential calls split
/// into two steps; each output lands in its own call by id).
fn two_step_group() -> ChatView {
    let mut v = ChatView::default();
    for (id, cmd, out) in [("a", "echo A", "A-out"), ("b", "echo B", "B-out")] {
        v.apply(&SessionEvent::ToolStart {
            id: id.into(),
            name: "bash".into(),
            input: serde_json::json!({"command": cmd}),
        });
        v.apply(&SessionEvent::ToolEnd {
            id: id.into(),
            name: "bash".into(),
            output: out.into(),
            is_error: false,
            images: Vec::new(),
        });
    }
    v
}

/// Open both of the group's steps; returns the view.
fn fully_open(mut v: ChatView) -> ChatView {
    v.toggle_tool_call_at(0, 0); // step 1 open
    v.toggle_tool_call_at(0, 2); // step 2 open (walk grew by call 0's row)
    v
}

fn expanded(v: &ChatView) -> Vec<bool> {
    match v.blocks.first() {
        Some(ChatBlock::StepGroup { steps }) => steps
            .iter()
            .flat_map(|s| s.calls.iter().map(|c| c.expanded))
            .collect(),
        other => panic!("expected a StepGroup first, got {other:?}"),
    }
}

fn flatten_text(v: &ChatView) -> Vec<String> {
    v.flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect()
}

#[test]
fn toggle_step_row_opens_only_that_step() {
    let mut v = two_step_group();

    // Walk is [Step(1), Step(2)]: target 0 opens step 1 only.
    v.toggle_tool_call_at(0, 0);
    let rows = flatten_text(&v);
    let joined = rows.join("\n");
    assert!(
        joined.contains("echo A"),
        "step 1 call row visible: {joined}"
    );
    assert!(
        !joined.contains("echo B"),
        "step 2 stays collapsed: {joined}"
    );

    // Second toggle collapses it again.
    v.toggle_tool_call_at(0, 0);
    assert!(
        !flatten_text(&v).join("\n").contains("echo A"),
        "closing the step hides its rows"
    );
}

#[test]
fn toggle_expands_only_that_call_in_open_step() {
    let mut v = fully_open(two_step_group());
    assert_eq!(expanded(&v), vec![false, false], "calls start collapsed");

    // Walk is [S1, call a, S2, call b]: target 1 is call a's header.
    v.toggle_tool_call_at(0, 1);
    assert_eq!(expanded(&v), vec![true, false], "only call a toggled");
    let joined = flatten_text(&v).join("\n");
    assert!(joined.contains("A-out"), "call a output visible: {joined}");
    assert!(
        !joined.contains("B-out"),
        "call b output stays hidden: {joined}"
    );
    assert!(
        flatten_text(&v).iter().any(|r| r.contains("echo B")),
        "call b's header row is still rendered"
    );

    // Second toggle collapses it again.
    v.toggle_tool_call_at(0, 1);
    assert_eq!(expanded(&v), vec![false, false]);
    assert!(!flatten_text(&v).join("\n").contains("A-out"));
}

#[test]
fn toggling_one_call_leaves_sibling_output_hidden() {
    let mut v = fully_open(two_step_group());
    // Walk is [S1, call a, S2, call b]: target 3 is call b's header.
    v.toggle_tool_call_at(0, 3);
    assert_eq!(expanded(&v), vec![false, true]);
    let joined = flatten_text(&v).join("\n");
    assert!(joined.contains("B-out"));
    assert!(!joined.contains("A-out"), "sibling output stays hidden");
}

#[test]
fn expanded_call_keeps_ladder_shape() {
    let mut v = fully_open(two_step_group());
    v.toggle_tool_call_at(0, 1); // expand call a
    let rows = flatten_text(&v);
    // marker, S1, a header, a output, separator, S2, b header, blank.
    assert_eq!(rows.len(), 8);
    assert!(rows[1].contains("Step(1)"));
    assert!(rows[2].contains("echo A"));
    assert!(rows[3].contains("A-out"));
    assert!(rows[4].is_empty(), "blank separator after expanded output");
    assert!(rows[5].contains("Step(2)"));
    assert!(rows[6].contains("echo B"));
}

#[test]
fn collapse_all_resets_expanded_calls() {
    let mut v = fully_open(two_step_group());
    v.toggle_tool_call_at(0, 1); // call a
    v.toggle_tool_call_at(0, 3); // call b
    assert_eq!(expanded(&v), vec![true, true]);

    v.collapse_all_collapsible();
    assert_eq!(
        expanded(&v),
        vec![false, false],
        "Ctrl+L must reset every call's expanded flag"
    );
    let joined = flatten_text(&v).join("\n");
    assert!(
        joined.contains("Step(1)") && joined.contains("Step(2)"),
        "Ctrl+L keeps the step rows rendered: {joined}"
    );
}

#[test]
fn out_of_range_toggle_is_a_noop() {
    let mut v = two_step_group();
    v.toggle_tool_call_at(9, 0);
    v.toggle_tool_call_at(0, 9);
    assert_eq!(
        expanded(&v),
        vec![false, false],
        "unrelated indices must not touch the calls"
    );
}

#[test]
fn step_and_call_rows_collected_while_visible() {
    let mut v = two_step_group();
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 2, "both step rows are always clickable");
    assert_eq!(headers[0].block_idx, 0);
    assert_eq!(headers[0].call_idx, 0);
    assert_eq!(headers[1].call_idx, 1);
    // Marker line is 0; the two step rows follow one row apart.
    assert_eq!(headers[0].header_line_idx + 1, headers[1].header_line_idx);

    // Opening step 1 inserts its call row into the walk and shifts step 2.
    v.toggle_tool_call_at(0, 0);
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 3);
    assert_eq!(headers[1].call_idx, 1, "call a's row joined the walk");
    assert_eq!(headers[2].call_idx, 2, "step 2 moved to flat index 2");
    assert_eq!(headers[0].header_line_idx + 2, headers[2].header_line_idx);

    // Expanding call a shifts step 2's row down by output + separator.
    v.toggle_tool_call_at(0, 1);
    let headers = v.tool_call_headers();
    assert_eq!(headers[0].header_line_idx + 4, headers[2].header_line_idx);

    // Closing the step removes its call row from the walk again.
    v.toggle_tool_call_at(0, 0);
    assert_eq!(v.tool_call_headers().len(), 2);
}
