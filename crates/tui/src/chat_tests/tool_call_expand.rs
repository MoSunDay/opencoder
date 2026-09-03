//! Three-level drill-down inside a `StepGroup`: clicking a rendered ladder
//! row toggles ONLY that target — the group row flips the whole group, a
//! step row flips that step, a calls aggregation row flips that step's call
//! list, a call header row toggles ONLY that call's output. Rows are
//! enumerated by the group's VISIBLE targets in render order.

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

fn group_open(v: &ChatView) -> bool {
    match v.blocks.first() {
        Some(ChatBlock::StepGroup { open, .. }) => *open,
        other => panic!("expected a StepGroup first, got {other:?}"),
    }
}

fn step_open(v: &ChatView) -> Vec<bool> {
    match v.blocks.first() {
        Some(ChatBlock::StepGroup { steps, .. }) => steps.iter().map(|s| s.open).collect(),
        other => panic!("expected a StepGroup first, got {other:?}"),
    }
}

fn calls_open(v: &ChatView) -> Vec<bool> {
    match v.blocks.first() {
        Some(ChatBlock::StepGroup { steps, .. }) => steps.iter().map(|s| s.calls_open).collect(),
        other => panic!("expected a StepGroup first, got {other:?}"),
    }
}

fn expanded(v: &ChatView) -> Vec<bool> {
    match v.blocks.first() {
        Some(ChatBlock::StepGroup { steps, .. }) => steps
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
fn toggle_group_row_opens_only_the_group() {
    // Zero clicks: walk = [Group]; the render is the closed group row only.
    let mut v = two_step_group();
    assert!(!group_open(&v));
    assert_eq!(v.tool_call_headers().len(), 1, "only the group row");
    let rows = flatten_text(&v);
    assert_eq!(rows.len(), 2, "group row + trailing blank: {rows:?}");
    assert!(rows[0].contains("\u{25b8} 2 steps"));

    // Target 0 = the group row: the group opens, both closed step rows
    // appear (no thinking → no Thinking rows, no aggregation rows yet).
    v.toggle_tool_call_at(0, 0);
    assert!(group_open(&v));
    assert_eq!(step_open(&v), vec![false, false]);
    let rows = flatten_text(&v);
    // group + S1 + S2 + blank.
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert!(rows[0].contains("\u{276f} 2 steps"), "open glyph: {rows:?}");
    assert!(rows[1].contains("\u{25b8} Step(1)"));
    assert!(rows[2].contains("\u{25b8} Step(2)"));

    // Re-toggling the group closes it again (steps keep their own state).
    v.toggle_tool_call_at(0, 0);
    assert!(!group_open(&v));
    assert_eq!(flatten_text(&v).len(), 2);
}

#[test]
fn toggle_step_row_opens_only_that_step() {
    let mut v = two_step_group();
    v.toggle_tool_call_at(0, 0); // group open; walk = [Group, S1, S2]

    // Walk is [Group, Step(1), Step(2)]: target 1 opens step 1 only.
    v.toggle_tool_call_at(0, 1);
    assert_eq!(step_open(&v), vec![true, false]);
    let rows = flatten_text(&v);
    let joined = rows.join("\n");
    assert!(
        joined.contains("\u{276f} Step(1)") && joined.contains("\u{25b8} Step(2)"),
        "sibling step stays closed: {joined}"
    );
    assert!(
        rows.iter().any(|r| r.contains("1 function call")),
        "opened step renders its calls aggregation row (singular): {rows:?}"
    );
    assert!(
        !joined.contains("echo A"),
        "the call list itself stays folded until the aggregation row opens"
    );

    // Closing the step removes the aggregation row again.
    v.toggle_tool_call_at(0, 1);
    assert_eq!(flatten_text(&v).len(), 4);
}

#[test]
fn toggle_calls_row_opens_only_that_steps_call_list() {
    let mut v = two_step_group();
    v.toggle_tool_call_at(0, 0); // group open
    v.toggle_tool_call_at(0, 1); // step 1 open; walk = [G, S1, Calls1, S2]

    // Target 2 = step 1's aggregation row: only step 1's list opens.
    v.toggle_tool_call_at(0, 2);
    assert_eq!(calls_open(&v), vec![true, false]);
    let rows = flatten_text(&v);
    assert!(
        rows.iter().any(|r| r.contains("\u{276f} 1 function call")),
        "open aggregation glyph: {rows:?}"
    );
    assert!(rows.iter().any(|r| r.contains("echo A")), "{rows:?}");
    assert!(
        rows.iter().any(|r| r.contains("\u{25b8} Step(2)")),
        "step 2 row still renders closed: {rows:?}"
    );
    assert!(
        !rows.iter().any(|r| r.contains("echo B")),
        "step 2's list stays folded: {rows:?}"
    );
    assert!(
        !rows.iter().any(|r| r.contains("A-out")),
        "call outputs stay hidden until the single call expands: {rows:?}"
    );
}

#[test]
fn toggle_call_header_expands_only_that_call_output() {
    let mut v = two_step_group();
    v.toggle_tool_call_at(0, 0); // group open
    v.toggle_tool_call_at(0, 1); // step 1 open
    v.toggle_tool_call_at(0, 2); // step 1 call list open; walk = [G, S1, C1, a, S2]
    assert_eq!(expanded(&v), vec![false, false], "calls start collapsed");

    // Target 3 = call a's header row.
    v.toggle_tool_call_at(0, 3);
    assert_eq!(expanded(&v), vec![true, false], "only call a toggled");
    let joined = flatten_text(&v).join("\n");
    assert!(joined.contains("A-out"), "call a output visible: {joined}");
    assert!(
        !joined.contains("B-out"),
        "call b output stays hidden: {joined}"
    );
    assert!(
        !joined.contains("echo B"),
        "step 2's call list stays folded: {joined}"
    );
}

#[test]
fn expanded_call_keeps_ladder_shape() {
    let mut v = two_step_group();
    v.toggle_tool_call_at(0, 0); // group
    v.toggle_tool_call_at(0, 1); // step 1
    v.toggle_tool_call_at(0, 2); // call list
    v.toggle_tool_call_at(0, 3); // expand call a
    let rows = flatten_text(&v);
    // group, S1, agg, a header, a output, separator, S2, blank.
    assert_eq!(rows.len(), 8, "{rows:?}");
    assert!(rows[1].contains("Step(1)"));
    assert!(rows[2].contains("1 function call"));
    assert!(rows[3].contains("echo A"));
    assert!(rows[4].contains("A-out"));
    assert!(rows[5].is_empty(), "blank separator after expanded output");
    assert!(rows[6].contains("Step(2)"));
}

#[test]
fn toggling_one_call_leaves_sibling_output_hidden() {
    let mut v = two_step_group();
    v.toggle_tool_call_at(0, 0); // group
    v.toggle_tool_call_at(0, 1); // step 1 open → walk [G, S1, C1, S2]
    v.toggle_tool_call_at(0, 2); // step 1 list open → [G, S1, C1, a, S2]
    v.toggle_tool_call_at(0, 3); // expand call a
    v.toggle_tool_call_at(0, 4); // step 2 open → [G, S1, C1, a, S2, C2]
    v.toggle_tool_call_at(0, 5); // step 2 list open → [..., b]
    v.toggle_tool_call_at(0, 6); // expand call b
    assert_eq!(expanded(&v), vec![true, true]);
    let joined = flatten_text(&v).join("\n");
    assert!(joined.contains("A-out") && joined.contains("B-out"));

    // Collapse just call b: its step/list state is untouched.
    v.toggle_tool_call_at(0, 6);
    assert_eq!(expanded(&v), vec![true, false]);
    assert_eq!(calls_open(&v), vec![true, true]);
    assert!(flatten_text(&v).join("\n").contains("A-out"));
}

#[test]
fn collapse_all_resets_every_ladder_level() {
    let mut v = two_step_group();
    v.toggle_tool_call_at(0, 0); // group
    v.toggle_tool_call_at(0, 1); // step 1
    v.toggle_tool_call_at(0, 2); // list
    v.toggle_tool_call_at(0, 3); // call a
    v.toggle_tool_call_at(0, 4); // step 2
    v.toggle_tool_call_at(0, 5); // list
    v.toggle_tool_call_at(0, 6); // call b
    assert!(group_open(&v));
    assert_eq!(step_open(&v), vec![true, true]);
    assert_eq!(calls_open(&v), vec![true, true]);
    assert_eq!(expanded(&v), vec![true, true]);

    v.collapse_all_collapsible();
    assert!(!group_open(&v), "Ctrl+L must close the group fold");
    assert_eq!(step_open(&v), vec![false, false]);
    assert_eq!(calls_open(&v), vec![false, false]);
    assert_eq!(
        expanded(&v),
        vec![false, false],
        "Ctrl+L must reset every call's expanded flag"
    );
    let rows = flatten_text(&v);
    assert_eq!(rows.len(), 2, "back to group row + blank: {rows:?}");
    assert!(rows[0].contains("\u{25b8} 2 steps"));
}

#[test]
fn out_of_range_toggle_is_a_noop() {
    let mut v = two_step_group();
    v.toggle_tool_call_at(9, 0);
    v.toggle_tool_call_at(0, 9);
    assert!(!group_open(&v));
    assert_eq!(
        expanded(&v),
        vec![false, false],
        "unrelated indices must not touch the calls"
    );
}

#[test]
fn ladder_rows_collected_while_visible() {
    let mut v = two_step_group();
    // Zero clicks: only the group row is a target (line 0).
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].call_idx, 0);
    assert_eq!(headers[0].header_line_idx, 0);

    // Group open: the two step rows join the walk.
    v.toggle_tool_call_at(0, 0);
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 3);
    assert_eq!(headers[1].call_idx, 1, "step 1's row");
    assert_eq!(headers[1].header_line_idx, 1);
    assert_eq!(headers[2].call_idx, 2, "step 2's row");
    assert_eq!(headers[2].header_line_idx, 2);

    // Step 1 open: its aggregation row joins between step 1 and step 2.
    v.toggle_tool_call_at(0, 1);
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 4);
    assert_eq!(headers[2].call_idx, 2, "aggregation row");
    assert_eq!(headers[2].header_line_idx, 2);
    assert_eq!(headers[3].call_idx, 3, "step 2 moved to flat index 3");
    assert_eq!(headers[3].header_line_idx, 3);

    // Call list open: call a's header row joins the walk.
    v.toggle_tool_call_at(0, 2);
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 5);
    assert_eq!(headers[3].call_idx, 3, "call a's row");
    assert_eq!(headers[3].header_line_idx, 3);
    assert_eq!(headers[4].call_idx, 4, "step 2 moved again");
    assert_eq!(headers[4].header_line_idx, 4);

    // Expanding call a shifts step 2's row by output + separator.
    v.toggle_tool_call_at(0, 3);
    let headers = v.tool_call_headers();
    assert_eq!(headers[0].header_line_idx + 6, headers[4].header_line_idx);

    // Closing the step removes its aggregation + call rows from the walk.
    v.toggle_tool_call_at(0, 1);
    assert_eq!(v.tool_call_headers().len(), 3);
}
