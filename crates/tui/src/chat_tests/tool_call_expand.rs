//! Turn → Step content/calls aggregate → Function-call result drill-down.

use super::super::*;

fn two_step_turn() -> ChatView {
    let mut view = ChatView::default();
    for (id, thinking, cmd, output) in [
        ("a", "inspect first", "echo A", "A-out"),
        ("b", "verify next", "echo B", "B-out"),
    ] {
        view.apply(&SessionEvent::ReasoningDelta(thinking.into()));
        view.apply(&SessionEvent::ToolStart {
            id: id.into(),
            name: "bash".into(),
            input: serde_json::json!({"command": cmd}),
        });
        view.apply(&SessionEvent::ToolEnd {
            id: id.into(),
            name: "bash".into(),
            output: output.into(),
            is_error: false,
            images: Vec::new(),
        });
    }
    if let Some(ChatBlock::StepGroup {
        progress_active, ..
    }) = view.blocks.first_mut()
    {
        *progress_active = false;
    }
    view
}

fn turn(view: &ChatView) -> (&Vec<Step>, bool) {
    match view.blocks.first() {
        Some(ChatBlock::StepGroup { steps, open, .. }) => (steps, *open),
        other => panic!("expected a StepGroup first, got {other:?}"),
    }
}

fn rows(view: &ChatView) -> Vec<String> {
    view.flatten()
        .iter()
        .map(|line| line.spans.iter().map(|span| span.content.clone()).collect())
        .collect()
}

#[test]
fn default_is_one_collapsed_turn_row() {
    let view = two_step_turn();
    let (steps, open) = turn(&view);
    assert_eq!(steps.len(), 2);
    assert!(!open);
    assert!(steps.iter().all(|step| !step.open && !step.calls_open));
    assert_eq!(view.tool_call_headers().len(), 1);
    assert_eq!(rows(&view), vec!["▸ 2 Steps", ""]);
}

#[test]
fn turn_click_reveals_only_steps() {
    let mut view = two_step_turn();
    view.toggle_tool_call_at(0, 0);

    let rendered = rows(&view);
    assert_eq!(rendered.len(), 4);
    assert!(rendered[0].contains("❯ 2 Steps"));
    assert!(rendered[1].contains("▸ Step(1)"));
    assert!(rendered[2].contains("▸ Step(2)"));
    assert!(!rendered.join("\n").contains("inspect first"));
    assert!(!rendered.join("\n").contains("echo A"));
}

#[test]
fn step_click_reveals_thinking_and_calls_aggregate() {
    let mut view = two_step_turn();
    view.toggle_tool_call_at(0, 0); // Turn → Steps.
    view.toggle_tool_call_at(0, 1); // Step(1) → Thinking + calls.

    let rendered = rows(&view);
    let joined = rendered.join("\n");
    assert!(joined.contains("💭 Thinking"));
    assert!(joined.contains("inspect first"));
    assert!(
        joined.contains("1 Function call"),
        "step must expose the calls aggregate: {joined}"
    );
    assert!(
        !joined.contains("echo A"),
        "call rows stay closed: {joined}"
    );
    assert!(!joined.contains("A-out"), "result stays closed: {joined}");

    let targets = view.tool_call_headers();
    assert_eq!(targets.len(), 4, "Turn, Step(1), Calls(1), Step(2)");
    assert_eq!(
        targets.iter().map(|h| h.call_idx).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
}

#[test]
fn calls_aggregate_reveals_rows_then_call_reveals_only_its_result() {
    let mut view = two_step_turn();
    view.toggle_tool_call_at(0, 0); // Turn.
    view.toggle_tool_call_at(0, 1); // Step(1).
    view.toggle_tool_call_at(0, 2); // Calls aggregate.

    let joined = rows(&view).join("\n");
    assert!(joined.contains("echo A"), "call row is visible: {joined}");
    assert!(!joined.contains("A-out"), "result stays closed: {joined}");

    view.toggle_tool_call_at(0, 3); // Function call a.

    let (steps, _) = turn(&view);
    assert!(steps[0].calls[0].expanded);
    assert!(!steps[1].calls[0].expanded);
    let joined = rows(&view).join("\n");
    assert!(joined.contains("A-out"));
    assert!(!joined.contains("B-out"));
}

#[test]
fn sibling_steps_and_calls_keep_independent_state() {
    let mut view = two_step_turn();
    view.toggle_tool_call_at(0, 0); // Turn.
    view.toggle_tool_call_at(0, 1); // Step(1).
    view.toggle_tool_call_at(0, 2); // Calls(1).
    view.toggle_tool_call_at(0, 3); // Call a.
    view.toggle_tool_call_at(0, 4); // Step(2).
    view.toggle_tool_call_at(0, 5); // Calls(2).
    view.toggle_tool_call_at(0, 6); // Call b.

    let (steps, _) = turn(&view);
    assert!(steps.iter().all(|step| step.open));
    assert!(steps.iter().all(|step| step.calls_open));
    assert!(steps.iter().all(|step| step.calls[0].expanded));

    view.toggle_tool_call_at(0, 6);
    let (steps, _) = turn(&view);
    assert!(steps[0].calls[0].expanded);
    assert!(!steps[1].calls[0].expanded);
}

#[test]
fn collapse_all_resets_all_three_levels() {
    let mut view = two_step_turn();
    for target in [0, 1, 2, 3, 4, 5, 6] {
        view.toggle_tool_call_at(0, target);
    }
    view.collapse_all_collapsible();

    let (steps, open) = turn(&view);
    assert!(!open);
    assert!(steps.iter().all(|step| !step.open && !step.calls_open));
    assert!(steps
        .iter()
        .flat_map(|step| &step.calls)
        .all(|call| !call.expanded));
    assert_eq!(rows(&view), vec!["▸ 2 Steps", ""]);
}

#[test]
fn header_line_indices_track_expanded_results() {
    let mut view = two_step_turn();
    view.toggle_tool_call_at(0, 0);
    view.toggle_tool_call_at(0, 1);
    view.toggle_tool_call_at(0, 2);
    view.toggle_tool_call_at(0, 3);

    let rendered = rows(&view);
    let headers = view.tool_call_headers();
    assert_eq!(headers.len(), 5, "Turn + Step/Calls/call + Step");
    for header in headers {
        let text = &rendered[header.header_line_idx];
        assert!(
            text.contains("Steps")
                || text.contains("Step(")
                || text.contains("Function call")
                || text.contains("echo"),
            "hit target points at non-clickable row {text:?}"
        );
    }
}

#[test]
fn out_of_range_toggle_is_a_noop() {
    let mut view = two_step_turn();
    view.toggle_tool_call_at(9, 0);
    view.toggle_tool_call_at(0, 9);
    assert!(!turn(&view).1);
}
