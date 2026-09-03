//! Regression tests for the sidecar panel storage split: the `/sidecar`
//! panel lives in the `ChatView::sidecar` field, never in `blocks`. Opening
//! the panel while the main task is streaming must not split an in-flight
//! Thinking/Assistant block nor break the tool-group tail merge — the old
//! placeholder-block-in-`blocks` design broke every `blocks.last_mut()`
//! invariant in `chat_stream.rs` the moment it was pushed.

use super::*;

/// Open the panel on `v` the way `/sidecar` does, returning the dummy ask
/// channel so a test can also drive `exit_panel`.
fn open_panel(v: &mut ChatView) -> tokio::sync::mpsc::Sender<crate::sidecar_ui::SidecarCmd> {
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::sidecar_ui::SidecarCmd>(1);
    crate::sidecar_ui::enter_panel(v, &tx);
    tx
}

/// Number of step groups currently in the transcript.
fn step_groups(v: &ChatView) -> usize {
    v.blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::StepGroup { .. }))
        .count()
}

/// Steps of the first step group.
fn first_steps(v: &ChatView) -> &Vec<Step> {
    v.blocks
        .iter()
        .find_map(|b| match b {
            ChatBlock::StepGroup { steps, .. } => Some(steps),
            _ => None,
        })
        .expect("expected a step group")
}

/// Concatenated thinking text of the first group's steps, in order.
fn thinking_text(v: &ChatView) -> String {
    first_steps(v)
        .iter()
        .map(|step| step.thinking_raw.as_str())
        .collect()
}

/// Reasoning deltas flowing either side of `/sidecar` panel entry must land
/// in ONE step of ONE group (live reasoning streams into the ladder): the
/// panel must not split the in-flight step, and exit must leave the
/// transcript exactly as it was.
#[test]
fn reasoning_delta_keeps_one_thinking_block_across_panel_entry() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("part one ".into()));
    let tx = open_panel(&mut v);
    assert!(v.sidecar.is_some(), "panel is stored on the view field");

    v.apply(&SessionEvent::ReasoningDelta("part two".into()));
    assert_eq!(step_groups(&v), 1, "panel entry must not open a new group");
    assert_eq!(
        first_steps(&v).len(),
        1,
        "panel entry must not split the step"
    );
    assert_eq!(thinking_text(&v), "part one part two");

    crate::sidecar_ui::exit_panel(&mut v, &tx);
    assert!(v.sidecar.is_none(), "exit clears the panel field");
    assert_eq!(step_groups(&v), 1, "exit leaves the transcript untouched");
    assert_eq!(thinking_text(&v), "part one part two");
}

/// Text deltas flowing either side of `/sidecar` panel entry must land in
/// ONE Assistant block, surviving enter + exit with both fragments.
#[test]
fn text_delta_keeps_one_assistant_block_across_panel_entry() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("answer one ".into()));
    let tx = open_panel(&mut v);
    assert!(v.sidecar.is_some(), "panel is stored on the view field");

    v.apply(&SessionEvent::TextDelta("answer two".into()));
    assert_eq!(
        v.blocks
            .iter()
            .filter(|b| matches!(b, ChatBlock::Assistant { .. }))
            .count(),
        1,
        "panel entry must not split the Assistant block"
    );

    crate::sidecar_ui::exit_panel(&mut v, &tx);
    assert!(v.sidecar.is_none(), "exit clears the panel field");
    assert_eq!(v.blocks.len(), 1, "exit leaves the transcript untouched");
    match &v.blocks[0] {
        ChatBlock::Assistant { raw, .. } => assert_eq!(raw, "answer one answer two"),
        other => panic!("expected a single Assistant block, got {other:?}"),
    }
}

/// Interleaved reasoning + text around panel entry: the panel must neither
/// seal the open Thinking nor split/reorder anything — the transcript must
/// be identical to the same stream WITHOUT a panel entry. (Every Say closes
/// its own sub-turn — `1 Turn = n Steps + Say` — so the interleaved round
/// settles as two ladder/Say pairs, panel or not.)
#[test]
fn panel_entry_does_not_seal_or_split_thinking_before_assistant() {
    let mut baseline = ChatView::default();
    baseline.apply(&SessionEvent::ReasoningDelta("think".into()));
    baseline.apply(&SessionEvent::TextDelta("answer".into()));
    baseline.apply(&SessionEvent::ReasoningDelta(" think2".into()));
    baseline.apply(&SessionEvent::TextDelta(" answer2".into()));
    baseline.finalize_assistant();

    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    v.apply(&SessionEvent::TextDelta("answer".into()));
    let tx = open_panel(&mut v);
    assert!(v.sidecar.is_some(), "panel is stored on the view field");
    v.apply(&SessionEvent::ReasoningDelta(" think2".into()));
    v.apply(&SessionEvent::TextDelta(" answer2".into()));
    v.finalize_assistant();

    // Live reasoning never leaves the ladder and each Say closes its own
    // sub-turn: the interleaved round settles as
    // [StepGroup, Assistant, StepGroup, Assistant] — nothing extra.
    assert_eq!(
        v.blocks.len(),
        4,
        "[Group, Say, Group, Say] exactly — nothing extra"
    );
    assert!(
        matches!(v.blocks.first(), Some(ChatBlock::StepGroup { .. })),
        "the step group stays in front"
    );
    assert_eq!(
        step_groups(&v),
        2,
        "the pairing contract's two ladders — panel entry adds none"
    );
    assert_eq!(
        first_steps(&v).len(),
        1,
        "each turn's reasoning stays in one step"
    );
    assert!(
        matches!(
            v.blocks.last(),
            Some(ChatBlock::Assistant { done: true, .. })
        ),
        "exactly one finalized Assistant at the tail"
    );
    let all_thinking: String = v
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::StepGroup { steps, .. } => Some(
                steps
                    .iter()
                    .map(|step| step.thinking_raw.as_str())
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect();
    assert!(
        all_thinking.contains("think") && all_thinking.contains("think2"),
        "both reasoning fragments survive, got {all_thinking:?}"
    );
    let answers: Vec<&str> = v
        .blocks
        .iter()
        .filter_map(|block| match block {
            ChatBlock::Assistant { raw, .. } => Some(raw.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        answers,
        ["answer", " answer2"],
        "each Say keeps its own answer fragment"
    );
    // The core regression pin: panel entry perturbed nothing.
    assert_eq!(
        v.blocks, baseline.blocks,
        "panel entry must not change the transcript vs a panel-free stream"
    );

    crate::sidecar_ui::exit_panel(&mut v, &tx);
    assert!(v.sidecar.is_none(), "exit clears the panel field");
}

/// Two `ToolStart`s either side of panel entry must end in ONE trailing
/// `StepGroup` holding both calls: the panel must not split the run.
#[test]
fn tool_start_merges_into_the_tail_group_across_panel_entry() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo one"}),
    });
    let _tx = open_panel(&mut v);
    assert!(v.sidecar.is_some(), "panel is stored on the view field");
    v.apply(&SessionEvent::ToolStart {
        id: "t2".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo two"}),
    });

    assert_eq!(v.blocks.len(), 1, "no split around the panel entry");
    match &v.blocks[0] {
        ChatBlock::StepGroup { steps, .. } => {
            let calls: Vec<_> = steps.iter().flat_map(|s| s.calls.iter()).collect();
            assert_eq!(calls.len(), 2, "both calls stay in one group");
            assert_eq!(calls[0].id, "t1");
            assert_eq!(calls[1].id, "t2");
        }
        other => panic!("expected a single StepGroup, got {other:?}"),
    }
}

/// The echoed question anchors the panel's turn: the sidecar's step ladder
/// must render BELOW the prompt. Regression: `echo_question` pushed the User
/// block without re-anchoring `turn_block_start`, so the first reasoning
/// delta inserted the StepGroup at the turn floor 0 — ABOVE the echo.
#[test]
fn panel_ladder_lands_below_the_echoed_question() {
    let mut v = ChatView::default();
    let _tx = open_panel(&mut v);
    crate::sidecar_ui::echo_question(&mut v, "总结这个函数");
    v.apply(&SessionEvent::SidecarStart {
        id: "sc-1".into(),
        question: "总结这个函数".into(),
    });
    v.apply(&SessionEvent::SidecarChild {
        id: "sc-1".into(),
        ev: Box::new(SessionEvent::ReasoningDelta("先看代码".into())),
    });
    let panel = v.sidecar.as_ref().expect("panel alive");
    let user_idx = panel
        .view
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::User { .. }))
        .expect("echoed question block");
    let group_idx = panel
        .view
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::StepGroup { .. }))
        .expect("panel step ladder");
    assert!(
        group_idx > user_idx,
        "ladder must render below the echoed question (user@{user_idx}, ladder@{group_idx})"
    );
    let text = block_text(&panel.view);
    let prompt = text.find("总结这个函数").expect("prompt text");
    let ladder = text.find("1 Step").expect("ladder marker");
    assert!(
        prompt < ladder,
        "flattened panel renders the prompt before the ladder: {text:?}"
    );
}
