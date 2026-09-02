//! Per-call expansion inside a `List`-state `ToolGroup` (Phase 1 of clickable
//! function calls): clicking a call's `▸ name args` row toggles ONLY that
//! call's output. `Results` stays fully expanded, `Collapsed` shows no call
//! rows, and Ctrl+L (`collapse_all_collapsible`) resets every call.

use super::super::*;

/// Two-call group built through the real event path (calls keep start order;
/// each output lands in its own call by id).
fn two_call_group() -> ChatView {
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

fn group_state(v: &ChatView) -> ToolGroupState {
    match v.blocks.first() {
        Some(ChatBlock::ToolGroup { state, .. }) => *state,
        other => panic!("expected a ToolGroup first, got {other:?}"),
    }
}

fn expanded(v: &ChatView) -> Vec<bool> {
    match v.blocks.first() {
        Some(ChatBlock::ToolGroup { calls, .. }) => calls.iter().map(|c| c.expanded).collect(),
        other => panic!("expected a ToolGroup first, got {other:?}"),
    }
}

fn flatten_text(v: &ChatView) -> Vec<String> {
    v.flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect()
}

#[test]
fn toggle_expands_only_that_call_in_list_state() {
    let mut v = two_call_group();
    v.cycle_tool_group_at(0);
    assert!(matches!(group_state(&v), ToolGroupState::List));
    assert_eq!(expanded(&v), vec![false, false], "calls start collapsed");

    v.toggle_tool_call_at(0, 0);
    assert_eq!(expanded(&v), vec![true, false], "only call 0 toggled");
    let rows = flatten_text(&v);
    let joined = rows.join("\n");
    assert!(joined.contains("A-out"), "call 0 output visible: {joined}");
    assert!(
        !joined.contains("B-out"),
        "call 1 output stays hidden: {joined}"
    );

    // The other call's header row is still rendered.
    assert!(
        rows.iter().any(|r| r.contains("echo B")),
        "call 1 header kept"
    );

    // Second toggle collapses it again.
    v.toggle_tool_call_at(0, 0);
    assert_eq!(expanded(&v), vec![false, false]);
    assert!(!flatten_text(&v).join("\n").contains("A-out"));
}

#[test]
fn toggling_one_call_leaves_sibling_output_hidden() {
    let mut v = two_call_group();
    v.cycle_tool_group_at(0);
    v.toggle_tool_call_at(0, 1);
    assert_eq!(expanded(&v), vec![false, true]);
    let joined = flatten_text(&v).join("\n");
    assert!(joined.contains("B-out"));
    assert!(!joined.contains("A-out"), "sibling output stays hidden");
}

#[test]
fn results_state_stays_fully_expanded_and_ignores_toggle() {
    let mut v = two_call_group();
    v.cycle_tool_group_at(0); // List
    v.cycle_tool_group_at(0); // Results
    v.toggle_tool_call_at(0, 0);
    let rows = flatten_text(&v);
    let joined = rows.join("\n");
    assert!(joined.contains("A-out") && joined.contains("B-out"));
    assert_eq!(
        expanded(&v),
        vec![false, false],
        "Results ignores the per-call flag"
    );
    // Line count identical to an untouched Results group.
    let mut baseline = two_call_group();
    baseline.cycle_tool_group_at(0);
    baseline.cycle_tool_group_at(0);
    assert_eq!(rows.len(), flatten_text(&baseline).len());
}

#[test]
fn collapsed_group_toggle_is_a_noop() {
    let mut v = two_call_group();
    assert!(matches!(group_state(&v), ToolGroupState::Collapsed));
    v.toggle_tool_call_at(0, 0);
    assert_eq!(expanded(&v), vec![false, false]);
    assert_eq!(v.flatten().len(), 1, "collapsed group renders one line");
}

#[test]
fn expanded_call_renders_output_and_separator_rows() {
    let mut v = two_call_group();
    v.cycle_tool_group_at(0);
    v.toggle_tool_call_at(0, 0);
    let rows = flatten_text(&v);
    // group line, a header, a output, separator, b header, trailing blank.
    assert_eq!(rows.len(), 6);
    assert!(rows[1].contains("echo A"));
    assert!(rows[2].contains("A-out"));
    assert!(rows[3].is_empty(), "blank separator after expanded output");
    assert!(rows[4].contains("echo B"));
}

#[test]
fn collapse_all_resets_expanded_calls() {
    let mut v = two_call_group();
    v.cycle_tool_group_at(0);
    v.toggle_tool_call_at(0, 0);
    v.toggle_tool_call_at(0, 1);
    assert_eq!(expanded(&v), vec![true, true]);

    v.collapse_all_collapsible();
    assert!(matches!(group_state(&v), ToolGroupState::Collapsed));
    assert_eq!(
        expanded(&v),
        vec![false, false],
        "Ctrl+L must reset every call's expanded flag"
    );
}

#[test]
fn out_of_range_toggle_is_a_noop() {
    let mut v = two_call_group();
    v.cycle_tool_group_at(0);
    v.toggle_tool_call_at(9, 0);
    v.toggle_tool_call_at(0, 9);
    assert_eq!(expanded(&v), vec![false, false]);
}

#[test]
fn call_header_rows_collected_only_in_list_state() {
    let mut v = two_call_group();
    assert!(v.tool_call_headers().is_empty(), "Collapsed: no call rows");

    v.cycle_tool_group_at(0); // List
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 2);
    assert_eq!(headers[0].block_idx, 0);
    assert_eq!(headers[0].call_idx, 0);
    assert_eq!(headers[1].call_idx, 1);
    // Group line is row 0; call headers follow it one row apart while both
    // calls are collapsed.
    assert_eq!(headers[0].header_line_idx + 1, headers[1].header_line_idx);

    // Expanding call 0 shifts call 1's header down by output + separator.
    v.toggle_tool_call_at(0, 0);
    let headers = v.tool_call_headers();
    assert_eq!(headers[0].header_line_idx + 3, headers[1].header_line_idx);

    v.cycle_tool_group_at(0); // Results
    assert!(
        v.tool_call_headers().is_empty(),
        "Results: no per-call toggle"
    );
}
