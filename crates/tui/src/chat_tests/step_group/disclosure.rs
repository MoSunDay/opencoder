use super::*;

fn disclosure(view: &ChatView) -> (bool, Vec<(bool, bool, Vec<bool>)>) {
    view.blocks
        .iter()
        .find_map(|block| match block {
            ChatBlock::StepGroup { open, steps, .. } => Some((
                *open,
                steps
                    .iter()
                    .map(|step| {
                        (
                            step.open,
                            step.calls_open,
                            step.calls.iter().map(|call| call.expanded).collect(),
                        )
                    })
                    .collect(),
            )),
            _ => None,
        })
        .expect("expected a step group")
}

#[test]
fn new_output_never_closes_user_opened_ladder_levels() {
    // Under live disclosure the ladder AUTO-opens at its running position;
    // a click on an already-open level therefore folds/pins it instead of
    // opening it. The user takes ownership of a level by cycling it
    // (fold + reopen = same visible state, now `user_touched`), after which
    // no streaming event may move that level again — new output never
    // closes a user-opened level, and new output never reopens a
    // user-closed one.
    let mut view = ChatView::default();
    view.apply(&SessionEvent::ReasoningDelta("first thought".into()));
    // Pin Turn + Step(1) in their auto-open state.
    view.toggle_tool_call_at(0, 0); // Turn → folded (pinned).
    view.toggle_tool_call_at(0, 0); // Turn → reopened (pinned).
    view.toggle_tool_call_at(0, 1); // Step(1) → folded (pinned).
    view.toggle_tool_call_at(0, 1); // Step(1) → reopened (pinned).

    view.apply(&SessionEvent::ReasoningDelta(" grows".into()));
    assert_eq!(disclosure(&view), (true, vec![(true, true, vec![])]));

    view.apply(&SessionEvent::LlmRoundEnd);
    assert_eq!(
        disclosure(&view),
        (true, vec![(true, true, vec![])]),
        "round finalization must not close the user-open turn or step"
    );

    view.apply(&SessionEvent::ToolStart {
        id: "call-1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    view.toggle_tool_call_at(0, 3); // Exact call result → expanded (pins the step).
    view.apply(&SessionEvent::ToolEnd {
        id: "call-1".into(),
        name: "bash".into(),
        output: "new output".into(),
        is_error: false,
        images: Vec::new(),
    });
    assert_eq!(
        disclosure(&view),
        (true, vec![(true, true, vec![true])]),
        "tool output must preserve all pinned ancestors and its result fold"
    );

    view.apply(&SessionEvent::ReasoningDelta("next round".into()));
    assert_eq!(
        disclosure(&view),
        (true, vec![(true, true, vec![true]), (true, true, vec![])]),
        "the fresh step takes over the open running position; the pinned \
step above keeps its disclosure untouched"
    );
    view.apply(&SessionEvent::LlmRoundEnd);
    assert_eq!(
        disclosure(&view),
        (true, vec![(true, true, vec![true]), (true, true, vec![])]),
        "round finalization must keep the entire ladder at its owned state"
    );

    view.toggle_tool_call_at(0, 0); // User explicitly closes the Turn.
    view.apply(&SessionEvent::TextDelta("later output".into()));
    assert_eq!(
        disclosure(&view),
        (false, vec![(true, true, vec![true]), (false, false, vec![])]),
        "new output must respect a user close: only auto-managed levels settle"
    );
}
