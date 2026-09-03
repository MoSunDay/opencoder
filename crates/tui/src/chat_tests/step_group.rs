//! Step = one assistant round (thinking + that round's calls). This file
//! pins the step-shape guarantees that the other chat_tests only touch
//! obliquely: the live thinking-folding path, replay's `coalesce_steps`
//! fold, and copy-mode chrome stripping for the two-level (step → call)
//! ladder under the static `≡ N steps` marker.

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
    // only visible once both the group and the step are opened.
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
        "trailing thinking must not stay a standalone block"
    );
    let steps = first_group(&v);
    assert_eq!(steps.len(), 1);
    let body = steps[0]
        .thinking
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect::<String>();
    assert!(body.contains("reading the layout"));

    if let ChatBlock::StepGroup { steps } = &mut v.blocks[0] {
        steps[0].open = true;
    }
    let flat = lines(&v);
    assert!(
        flat.iter().any(|l| l.contains("Thinking")),
        "opened step must render its thinking behind a Thinking header: {flat:?}"
    );
    assert!(flat.iter().any(|l| l.contains("reading the layout")));
    // Sanity: the static marker is the plain step count (thinking excluded).
    assert_eq!(flat[0], "\u{2261} 1 step");
}

#[test]
fn thinking_before_answer_text_stays_standalone() {
    // Strictly trailing: a round that streams answer text after its
    // reasoning keeps that Thinking block in the flow; the later call
    // opens a thinking-less step.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("mulling".into()));
    v.apply(&SessionEvent::TextDelta("here is the plan".into()));
    call_tool(&mut v, "t1");

    assert!(
        v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "non-trailing thinking must survive as its own block"
    );
    assert!(first_group(&v)[0].thinking.is_empty());
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
fn replay_folds_thinking_into_the_first_step() {
    let (a1, t1) = replay_tool_round("a1", "t1", Some("plan the edit"));
    let chat = replay_messages("act", &[a1, t1]);

    assert!(
        !chat
            .blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "replay must fold thinking into the step, not leave it in the flow"
    );
    let steps = first_group(&chat);
    let body: String = steps[0]
        .thinking
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect();
    assert!(body.contains("plan the edit"));
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
fn copy_mode_drops_step_chrome_but_keeps_content() {
    use crate::copy_mode::clean::clean_line;

    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("copy me".into()));
    call_tool(&mut v, "t1");
    // Open the ladder: step → single call output.
    if let ChatBlock::StepGroup { steps } = &mut v.blocks[0] {
        steps[0].open = true;
    }
    v.toggle_tool_call_at(0, 1);

    let mut saw_step_row = false;
    let mut saw_thinking_header = false;
    let mut saw_group_row = false;
    let mut copied = Vec::new();
    for line in v.flatten() {
        let text: String = line.spans.iter().map(|s| s.content.clone()).collect();
        match clean_line(&line) {
            None => {
                assert!(
                    text.contains("Step(")
                        || text.contains("Thinking")
                        || text.is_empty()
                        || text.starts_with(' ')
                        || text.contains("step"),
                    "only decoration rows may drop: {text:?}"
                );
                if text.contains("Step(") {
                    saw_step_row = true;
                }
                if text.contains("Thinking") {
                    saw_thinking_header = true;
                }
                if text.contains("step") {
                    saw_group_row = true;
                }
            }
            Some(payload) => copied.push(payload),
        }
    }
    assert!(saw_step_row, "precondition: step rows were rendered");
    assert!(
        saw_thinking_header,
        "precondition: Thinking header was rendered"
    );
    assert!(saw_group_row, "precondition: the group marker was rendered");
    let joined = copied.join("\n");
    assert!(joined.contains("copy me"), "thinking body is copyable");
    assert!(joined.contains("t1-out"), "call output is copyable");
    assert!(!joined.contains("Step("), "step labels are chrome");
    assert!(!joined.contains("1 step"), "group markers are chrome");
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
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.clone())
                    .collect::<String>()
            })
            .collect()
    };

    // Live: the synthetic step joins the trailing group — its finished call
    // forces a fresh step, never a second group.
    let mut live = ChatView::default();
    call_tool(&mut live, "t1");
    live.apply(&SessionEvent::ToolEnd {
        id: "ghost".into(),
        name: "bash".into(),
        output: "ghost-out".into(),
        is_error: false,
        images: Vec::new(),
    });
    let live_groups = groups(&live);
    assert_eq!(live_groups.len(), 1, "orphan must not open a second group");
    assert_eq!(live_groups[0].len(), 2, "finished call forces a fresh step");
    assert!(
        tail_output(live_groups[0]).contains("ghost-out"),
        "orphan output is kept"
    );

    // Replay of the same anomaly: the orphan ToolResult's synthetic group is
    // folded by `coalesce_steps` — one group, two steps, same output.
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
fn multi_round_turn_renders_marker_steps_and_answer_without_any_click() {
    // The user's core display requirement: after a multi-round turn the
    // static marker `≡ N steps`, EVERY step row, and the top-level final
    // answer block are all visible in the default (no-click) render.
    let mut v = ChatView::default();
    for id in ["t1", "t2"] {
        v.apply(&SessionEvent::ReasoningDelta(format!("think {id}")));
        call_tool(&mut v, id);
    }
    v.apply(&SessionEvent::TextDelta("all done".into()));
    v.apply(&SessionEvent::Done);

    let flat = lines(&v);
    assert!(
        flat.iter().any(|l| l.contains("\u{2261} 2 steps")),
        "static marker must render without a click: {flat:?}"
    );
    assert!(
        flat.iter().any(|l| l.contains("\u{25b8} Step(1)")),
        "step 1 row must render closed by default: {flat:?}"
    );
    assert!(
        flat.iter().any(|l| l.contains("\u{25b8} Step(2)")),
        "step 2 row must render closed by default: {flat:?}"
    );
    assert!(
        flat.iter().any(|l| l.contains("\u{276f} Say:")),
        "final answer block stays top-level and visible: {flat:?}"
    );
    // Closed steps still hide their content by default.
    assert!(
        !flat.iter().any(|l| l.contains("echo x")),
        "closed steps keep call rows hidden: {flat:?}"
    );
}
