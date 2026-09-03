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
    let mut view = ChatView::default();
    view.apply(&SessionEvent::ReasoningDelta("first thought".into()));
    view.toggle_tool_call_at(0, 0); // Turn.
    view.toggle_tool_call_at(0, 1); // Step(1).

    view.apply(&SessionEvent::ReasoningDelta(" grows".into()));
    assert_eq!(disclosure(&view), (true, vec![(true, false, vec![])]));

    view.apply(&SessionEvent::LlmRoundEnd);
    assert_eq!(
        disclosure(&view),
        (true, vec![(true, false, vec![])]),
        "round finalization must not close the open turn or step"
    );

    view.apply(&SessionEvent::ToolStart {
        id: "call-1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    view.toggle_tool_call_at(0, 2); // Function-call aggregate.
    view.toggle_tool_call_at(0, 3); // Exact call result.
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
        "tool output must preserve all opened ancestors and its result fold"
    );

    view.apply(&SessionEvent::ReasoningDelta("next round".into()));
    assert_eq!(
        disclosure(&view),
        (true, vec![(true, true, vec![true]), (false, false, vec![])]),
        "a new step may start closed but must not close previously opened content"
    );
    view.toggle_tool_call_at(0, 4); // Step(2).
    view.apply(&SessionEvent::LlmRoundEnd);
    assert_eq!(
        disclosure(&view),
        (true, vec![(true, true, vec![true]), (true, false, vec![])]),
        "settling a newly opened step must keep the entire ladder open"
    );

    view.toggle_tool_call_at(0, 0); // User explicitly closes the Turn.
    view.apply(&SessionEvent::TextDelta("later output".into()));
    assert!(!disclosure(&view).0, "new output must respect a user close");
}
