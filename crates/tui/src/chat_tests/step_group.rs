//! Step = one Thinking run plus all calls until the next Thinking. This file
//! pins the step-shape guarantees that the other chat_tests only touch
//! obliquely: the live thinking-absorption path, replay's `coalesce_steps`
//! fold, the zero-click collapsed ladder, and copy-mode chrome stripping
//! for the three-level (turn → step → function call) drill-down.

use super::super::*;

use crate::session_ui::replay_messages;
use opencoder_core::{ContentBlock, Message, Role};

fn lines(v: &ChatView) -> Vec<String> {
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

fn first_group_mut(v: &mut ChatView) -> &mut Vec<Step> {
    v.blocks
        .iter_mut()
        .find_map(|b| match b {
            ChatBlock::StepGroup { steps, .. } => Some(steps),
            _ => None,
        })
        .expect("expected a step group")
}

fn first_group(v: &ChatView) -> &Vec<Step> {
    v.blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::StepGroup { steps, .. } => Some(steps),
            _ => None,
        })
        .next()
        .expect("expected a step group")
}

fn call_tool(v: &mut ChatView, id: &str) {
    v.apply(&SessionEvent::ToolStart {
        id: id.into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: id.into(),
        name: "bash".into(),
        output: format!("{id}-out"),
        is_error: false,
        images: Vec::new(),
    });
}

#[test]
fn trailing_thinking_folds_into_the_step_not_the_flow() {
    // Live path: a Reasoning run that immediately precedes a tool call is
    // that round's step-thinking — it leaves the main block flow and is
    // only visible once the turn and the step are opened.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("reading the layout".into()));
    assert!(
        v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::StepGroup { steps, .. } if steps.len() == 1 && steps[0].calls.is_empty())),
        "reasoning starts in a call-less step"
    );
    call_tool(&mut v, "t1");

    assert!(
        !v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "thinking must never enter the main flow"
    );
    let body: String = first_group(&v)[0]
        .thinking
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect::<String>();
    assert!(body.contains("reading the layout"));

    // Zero clicks: only the closed group row renders.
    let flat = lines(&v);
    assert_eq!(flat.len(), 2, "closed group = group row + blank: {flat:?}");
    assert_eq!(flat[0], "\u{25b8} 1 Step  \u{280b} running ");

    // Group open, then step open: the step row renders, then its thinking
    // behind the Thinking header.
    if let ChatBlock::StepGroup { open, .. } = &mut v.blocks[0] {
        *open = true;
    }
    let flat = lines(&v);
    assert!(
        flat.iter().any(|l| l.contains("\u{25b8} Step(1)")),
        "opened group must list the closed step row: {flat:?}"
    );
    assert!(
        !flat.iter().any(|l| l.contains("reading the layout")),
        "step thinking stays hidden until the step opens"
    );

    first_group_mut(&mut v)[0].open = true;
    let flat = lines(&v);
    assert!(
        flat.iter().any(|l| l.contains("Thinking")),
        "opened step must render its thinking behind a Thinking header: {flat:?}"
    );
    assert!(flat.iter().any(|l| l.contains("reading the layout")));
}

#[test]
fn thinking_across_assistant_text_is_absorbed_into_the_step() {
    // The turn's own speech is never a boundary: a round that streamed
    // interim text between its reasoning and its tool call still folds that
    // thinking into the step — no top-level Thinking block survives.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("mulling".into()));
    v.apply(&SessionEvent::TextDelta("here is the plan".into()));
    call_tool(&mut v, "t1");

    assert!(
        !v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "a tool round's thinking must be absorbed even behind assistant text"
    );
    assert!(
        v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Assistant { .. })),
        "the interim speech itself stays in the flow"
    );
    let body: String = first_group(&v)[0]
        .thinking
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect::<String>();
    assert!(body.contains("mulling"), "got {body:?}");
}

#[test]
fn pure_text_turn_folds_thinking_into_a_call_less_step() {
    // A round with NO tool call still keeps its reasoning inside the ladder:
    // at run end the pending Thinking folds into a call-less step, so no
    // top-level Thinking block survives and Say stays the bare conclusion.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("just talking".into()));
    v.apply(&SessionEvent::TextDelta("the answer".into()));
    v.apply(&SessionEvent::Done);

    assert!(
        !v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "no standalone Thinking block survives a pure-text turn"
    );
    let groups: Vec<&Vec<Step>> = v
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::StepGroup { steps, .. } => Some(steps),
            _ => None,
        })
        .collect();
    assert_eq!(groups.len(), 1, "exactly one ladder for the turn");
    assert_eq!(groups[0].len(), 1, "one call-less step");
    assert!(groups[0][0].calls.is_empty());
    let body: String = groups[0][0]
        .thinking
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect();
    assert!(body.contains("just talking"), "got {body:?}");
    // The ladder sits before the Say; the group row counts the step.
    let group_idx = v
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::StepGroup { .. }))
        .unwrap();
    let say_idx = v
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::Assistant { .. }))
        .unwrap();
    assert!(group_idx < say_idx, "ladder before Say");
    let flat = lines(&v);
    assert!(
        flat.iter().any(|l| l.contains("1 Step")),
        "group row counts the call-less step: {flat:?}"
    );
}

#[test]
fn tool_turn_final_round_folds_its_say_round_thinking() {
    // The turn's FINAL round (thinking + Say, no tool call) folds into the
    // trailing group as one more call-less step — the ladder never leaks a
    // top-level Thinking block, and the Say stays outside the ladder.
    let mut v = ChatView::default();
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::ReasoningDelta("final mull".into()));
    v.apply(&SessionEvent::TextDelta("conclusion".into()));
    v.apply(&SessionEvent::Done);

    assert!(
        !v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "final-round thinking must not leak above the ladder"
    );
    let steps = first_group(&v);
    assert_eq!(steps.len(), 2, "tool round + Say round");
    assert!(!steps[0].calls.is_empty());
    assert!(steps[1].calls.is_empty(), "Say round is call-less");
    let body: String = steps[1]
        .thinking
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect::<String>();
    assert!(body.contains("final mull"), "got {body:?}");
}

#[test]
fn error_turn_folds_pending_thinking_into_the_ladder() {
    // An errored run must not strand a top-level Thinking block either.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("doomed mull".into()));
    v.apply(&SessionEvent::Error("boom".into()));

    assert!(
        !v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "error run folds pending thinking into the ladder"
    );
    let steps = first_group(&v);
    assert_eq!(steps.len(), 1);
    assert!(steps[0].calls.is_empty());
}

#[test]
fn user_echo_flushes_pre_boundary_thinking_into_the_ladder() {
    // A steer echo is a hard segment boundary: the pre-boundary thinking can
    // never be absorbed by a later ToolStart, so it folds right there.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("pre-steer mull".into()));
    v.apply(&SessionEvent::SteerConsumed {
        seq: 1,
        text: "steered prompt".into(),
    });
    assert!(
        !v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "pre-boundary thinking folds at the echo, not later"
    );
    let steps = first_group(&v);
    assert_eq!(steps.len(), 1);
    assert!(steps[0].calls.is_empty());
}

#[test]
fn mid_conversation_flush_lands_after_the_user_prompt() {
    // Turn two of a real conversation: the walk-back must place the ladder
    // AFTER the user's prompt (same segment), never above it.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("first answer".into()));
    v.apply(&SessionEvent::Done);
    // App layer pushes the user echo directly (no SessionEvent for it).
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("second prompt"),
    });
    let prompt_idx = v.blocks.len() - 1;
    v.begin_turn();
    v.apply(&SessionEvent::ReasoningDelta("mulling turn two".into()));
    v.apply(&SessionEvent::TextDelta("second answer".into()));
    v.apply(&SessionEvent::Done);

    assert!(
        !v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "no standalone Thinking block survives"
    );
    match &v.blocks[prompt_idx + 1] {
        ChatBlock::StepGroup { steps, .. } => {
            assert_eq!(steps.len(), 1);
            assert!(steps[0].calls.is_empty());
            let body: String = steps[0]
                .thinking
                .iter()
                .flat_map(|l| l.spans.iter())
                .map(|s| s.content.clone())
                .collect();
            assert!(body.contains("mulling turn two"));
        }
        other => panic!("expected ladder right after the prompt, got {other:?}"),
    }
}

#[test]
fn begin_turn_is_the_only_live_step_group_boundary() {
    let mut v = ChatView::default();
    call_tool(&mut v, "first");
    v.apply(&SessionEvent::TextDelta("say one".into()));
    v.apply(&SessionEvent::Done);
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("second prompt"),
    });
    v.begin_turn();
    call_tool(&mut v, "second");

    let groups: Vec<&Vec<Step>> = v
        .blocks
        .iter()
        .filter_map(|block| match block {
            ChatBlock::StepGroup { steps, .. } => Some(steps),
            _ => None,
        })
        .collect();
    assert_eq!(groups.len(), 2, "real user turns must not share a group");
    assert_eq!(groups[0][0].calls[0].id, "first");
    assert_eq!(groups[1][0].calls[0].id, "second");
}

#[path = "step_group/replay.rs"]
mod replay;

#[path = "step_group/disclosure.rs"]
mod disclosure;
