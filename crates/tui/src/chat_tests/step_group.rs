//! Step = one assistant round (thinking + that round's calls). This file
//! pins the step-shape guarantees that the other chat_tests only touch
//! obliquely: the live thinking-absorption path, replay's `coalesce_steps`
//! fold, the zero-click collapsed ladder, and copy-mode chrome stripping
//! for the three-level (group → step → calls list → single call) drill-down.

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
    // only visible once the group, the step (and, for calls, the call list)
    // are opened.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("reading the layout".into()));
    assert!(
        v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "precondition: reasoning starts as a Thinking block"
    );
    call_tool(&mut v, "t1");

    assert!(
        !v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "thinking must leave the main flow once the round opens a step"
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
    assert_eq!(flat[0], "\u{25b8} 1 step");

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
fn pure_text_turn_keeps_its_standalone_thinking_block() {
    // A round with NO tool call keeps its independent collapsible Thinking
    // block — there is no step to fold into, and the thinking is not lost.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("just talking".into()));
    v.apply(&SessionEvent::TextDelta("the answer".into()));
    v.apply(&SessionEvent::Done);

    assert!(
        v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "pure-text turn keeps a standalone Thinking block"
    );
    assert!(!v
        .blocks
        .iter()
        .any(|b| matches!(b, ChatBlock::StepGroup { .. })));
    let flat = lines(&v);
    assert!(
        flat.iter().any(|l| l.contains("Thinking")),
        "standalone thinking renders its own header: {flat:?}"
    );
}

fn replay_tool_round(asst_id: &str, tool_id: &str, reasoning: Option<&str>) -> (Message, Message) {
    let mut asst = Message::assistant(asst_id);
    if let Some(text) = reasoning {
        asst.blocks
            .push(ContentBlock::Reasoning { text: text.into() });
    }
    asst.blocks.push(ContentBlock::ToolUse {
        id: tool_id.into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    let mut tool_msg = Message::assistant(format!("{tool_id}-res"));
    tool_msg.role = Role::Tool;
    tool_msg.blocks = vec![ContentBlock::ToolResult {
        tool_use_id: tool_id.into(),
        content: format!("{tool_id}-out"),
        is_error: false,
        images: Vec::new(),
    }];
    (asst, tool_msg)
}

#[test]
fn replay_absorbs_thinking_behind_assistant_text_like_the_live_path() {
    // Replay's coalesce fold must cross Assistant blocks exactly like the
    // live path: [Thinking, Assistant(text), ToolUse] folds the thinking
    // into the step and keeps the assistant text top-level.
    let mut asst = Message::assistant("a1");
    asst.blocks.push(ContentBlock::Reasoning {
        text: "ponder replay".into(),
    });
    asst.blocks.push(ContentBlock::text("interim note"));
    asst.blocks.push(ContentBlock::ToolUse {
        id: "t1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    let mut tool_msg = Message::assistant("t1-res");
    tool_msg.role = Role::Tool;
    tool_msg.blocks = vec![ContentBlock::ToolResult {
        tool_use_id: "t1".into(),
        content: "t1-out".into(),
        is_error: false,
        images: Vec::new(),
    }];
    let chat = replay_messages("act", &[asst, tool_msg]);

    let groups: Vec<_> = chat
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::StepGroup { steps, .. } => Some(steps),
            _ => None,
        })
        .collect();
    assert_eq!(groups.len(), 1);
    let body: String = groups[0][0]
        .thinking
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect();
    assert!(
        body.contains("ponder replay"),
        "thinking behind assistant text must fold into the step: {body:?}"
    );
    assert!(
        !chat
            .blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "no top-level Thinking block survives next to a tool round"
    );
}

#[test]
fn replay_keeps_thinking_without_a_following_group_standalone() {
    let mut asst = Message::assistant("a1");
    asst.blocks.push(ContentBlock::Reasoning {
        text: "silent pondering".into(),
    });
    asst.blocks.push(ContentBlock::text("the answer"));
    let chat = replay_messages("act", &[asst]);

    assert!(
        chat.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "thinking with no following step group keeps its own rendering path"
    );
    assert!(!chat
        .blocks
        .iter()
        .any(|b| matches!(b, ChatBlock::StepGroup { .. })));
}

#[test]
fn replay_merges_adjacent_tool_rounds_into_one_group() {
    // Two consecutive assistant tool rounds (no user text in between) must
    // collapse into ONE StepGroup carrying one step per round.
    let (a1, t1) = replay_tool_round("a1", "t1", None);
    let (a2, t2) = replay_tool_round("a2", "t2", None);
    let chat = replay_messages("act", &[a1, t1, a2, t2]);

    let groups: Vec<_> = chat
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::StepGroup { steps, .. } => Some(steps),
            _ => None,
        })
        .collect();
    assert_eq!(groups.len(), 1, "adjacent groups merge: {groups:?}");
    assert_eq!(groups[0].len(), 2, "one step per round");
    let ids: Vec<&str> = groups[0]
        .iter()
        .flat_map(|s| s.calls.iter())
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, ["t1", "t2"]);
}

#[test]
fn copy_mode_drops_ladder_chrome_but_keeps_call_content() {
    use crate::copy_mode::clean::clean_line;

    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("copy me".into()));
    call_tool(&mut v, "t1");
    // Open the whole ladder: group → step → call list → single call output.
    if let ChatBlock::StepGroup { open, steps, .. } = &mut v.blocks[0] {
        *open = true;
        steps[0].open = true;
        steps[0].calls_open = true;
    }
    v.toggle_tool_call_at(0, 3); // walk: [Group, Step, Calls, Call] → call

    let mut saw_group_row = false;
    let mut saw_step_row = false;
    let mut saw_calls_row = false;
    let mut saw_thinking_header = false;
    let mut copied = Vec::new();
    for line in v.flatten() {
        let text: String = line.spans.iter().map(|s| s.content.clone()).collect();
        match clean_line(&line) {
            None => {
                if text.contains("step") && !text.contains("Step(") {
                    saw_group_row = true;
                }
                if text.contains("Step(") {
                    saw_step_row = true;
                }
                if text.contains("function call") {
                    saw_calls_row = true;
                }
                if text.contains("Thinking") {
                    saw_thinking_header = true;
                }
            }
            Some(payload) => copied.push(payload),
        }
    }
    assert!(saw_group_row, "precondition: the group row was rendered");
    assert!(saw_step_row, "precondition: the step row was rendered");
    assert!(
        saw_calls_row,
        "precondition: the calls aggregation row was rendered"
    );
    assert!(
        saw_thinking_header,
        "precondition: the Thinking header was rendered"
    );
    let joined = copied.join("\n");
    assert!(joined.contains("copy me"), "thinking body is copyable");
    assert!(joined.contains("t1-out"), "call output is copyable");
    assert!(
        joined.contains("\u{25b8} bash"),
        "the call header row is content, not chrome: {joined:?}"
    );
    assert!(!joined.contains("Step("), "step labels are chrome");
    assert!(!joined.contains("1 step"), "group rows are chrome");
    assert!(
        !joined.contains("function call"),
        "calls aggregation rows are chrome"
    );
    assert!(!joined.contains("Thinking"), "thinking header is chrome");
}

#[test]
fn orphan_tool_end_joins_the_trailing_group_like_replay() {
    // Orphan ToolEnd (lost ToolStart) must fold into the trailing group —
    // the same fold replay's `coalesce_steps` applies to adjacent groups —
    // so an anomalous transcript renders identically before and after
    // resume.
    fn groups(v: &ChatView) -> Vec<&Vec<Step>> {
        v.blocks
            .iter()
            .filter_map(|b| match b {
                ChatBlock::StepGroup { steps, .. } => Some(steps),
                _ => None,
            })
            .collect()
    }
    let tail_output = |steps: &[Step]| -> String {
        steps
            .last()
            .unwrap()
            .calls
            .last()
            .unwrap()
            .output
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect()
    };
    let (a1, t1) = replay_tool_round("a1", "t1", None);
    let mut ghost = Message::assistant("ghost-res");
    ghost.role = Role::Tool;
    ghost.blocks = vec![ContentBlock::ToolResult {
        tool_use_id: "ghost".into(),
        content: "ghost-out".into(),
        is_error: false,
        images: Vec::new(),
    }];
    let chat = replay_messages("act", &[a1, t1, ghost]);
    let replay_groups = groups(&chat);
    assert_eq!(replay_groups.len(), 1, "resume folds the orphan too");
    assert_eq!(replay_groups[0].len(), 2);
    assert!(tail_output(replay_groups[0]).contains("ghost-out"));
}

#[test]
fn multi_round_turn_zero_click_shows_only_group_row_and_say() {
    // The user's core display requirement: with no clicks a tool turn shows
    // exactly ONE clickable group row (collapsed glyph) plus the top-level
    // final Say; steps, calls and thinking are all folded away.
    let mut v = ChatView::default();
    for id in ["t1", "t2"] {
        v.apply(&SessionEvent::ReasoningDelta(format!("think {id}")));
        call_tool(&mut v, id);
    }
    v.apply(&SessionEvent::TextDelta("all done".into()));
    v.apply(&SessionEvent::Done);

    let flat = lines(&v);
    assert!(
        flat.iter().any(|l| l.contains("\u{25b8} 2 steps")),
        "collapsed group row with the closed prefix: {flat:?}"
    );
    assert!(
        flat.iter().any(|l| l.contains("\u{276f} Say:")),
        "final answer block stays top-level and visible: {flat:?}"
    );
    assert!(
        !flat.iter().any(|l| l.contains("Step(")),
        "step rows are hidden until the group opens: {flat:?}"
    );
    assert!(
        !flat.iter().any(|l| l.contains("echo x")),
        "call headers stay hidden: {flat:?}"
    );
    assert!(
        !flat.iter().any(|l| l.contains("think t")),
        "step thinking stays hidden: {flat:?}"
    );
    assert!(
        !flat.iter().any(|l| l.contains("-out")),
        "call outputs stay hidden: {flat:?}"
    );
}
