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
fn streaming_never_moves_user_set_levels() {
    // Disclosure is ALWAYS user-controlled: the ladder is born folded and
    // no streaming event opens or closes a level. Whatever state the user
    // sets — open or folded — persists through deltas, round finalization,
    // tool output, a new running step, and even the Say settle.
    let mut view = ChatView::default();
    view.apply(&SessionEvent::ReasoningDelta("first thought".into()));
    assert_eq!(
        disclosure(&view),
        (false, vec![(false, false, vec![])]),
        "the ladder is born folded; the running step only previews"
    );

    view.toggle_tool_call_at(0, 0); // Turn -> open.
    view.toggle_tool_call_at(0, 1); // Step(1) -> open.
    assert_eq!(disclosure(&view), (true, vec![(true, false, vec![])]));

    view.apply(&SessionEvent::ReasoningDelta(" grows".into()));
    assert_eq!(
        disclosure(&view),
        (true, vec![(true, false, vec![])]),
        "deltas keep the user's state"
    );

    view.apply(&SessionEvent::LlmRoundEnd);
    assert_eq!(
        disclosure(&view),
        (true, vec![(true, false, vec![])]),
        "round finalization must not close the user-open turn or step"
    );

    view.apply(&SessionEvent::ToolStart {
        id: "call-1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    view.toggle_tool_call_at(0, 2); // Calls aggregate -> open.
    view.toggle_tool_call_at(0, 3); // Call row -> expanded.
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
        "tool output must preserve the user's levels and the result fold"
    );

    view.apply(&SessionEvent::ReasoningDelta("next round".into()));
    assert_eq!(
        disclosure(&view),
        (true, vec![(true, true, vec![true]), (false, false, vec![])]),
        "the fresh running step opens NOTHING - it only takes the preview slot"
    );

    view.toggle_tool_call_at(0, 0); // User explicitly closes the Turn.
    view.apply(&SessionEvent::TextDelta("later output".into()));
    assert_eq!(
        disclosure(&view),
        (
            false,
            vec![(true, true, vec![true]), (false, false, vec![])]
        ),
        "the Say settle must respect the user close: every level keeps its state"
    );
}
