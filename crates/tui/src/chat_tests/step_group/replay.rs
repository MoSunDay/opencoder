use super::*;

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
fn replay_folds_thinking_without_a_following_group_into_the_ladder() {
    // Old-format transcripts (pre-ladder flush) carry the pure-text round's
    // Thinking standalone; replay folds it into a call-less step exactly
    // like the live path, so resumed sessions never re-leak thinking.
    let mut asst = Message::assistant("a1");
    asst.blocks.push(ContentBlock::Reasoning {
        text: "silent pondering".into(),
    });
    asst.blocks.push(ContentBlock::text("the answer"));
    let chat = replay_messages("act", &[asst]);

    assert!(
        !chat
            .blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "old-format pure-text rounds fold into the ladder on replay"
    );
    let groups: Vec<&Vec<Step>> = chat
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::StepGroup { steps, .. } => Some(steps),
            _ => None,
        })
        .collect();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].len(), 1);
    assert!(groups[0][0].calls.is_empty());
    let body: String = groups[0][0]
        .thinking
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect::<String>();
    assert!(body.contains("silent pondering"), "got {body:?}");
}

#[test]
fn replay_merges_adjacent_tool_rounds_into_one_group() {
    // Two consecutive assistant tool rounds (no user text in between) must
    // collapse into ONE StepGroup and, without new Thinking, ONE Step.
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
    assert_eq!(groups[0].len(), 1, "provider rounds do not create steps");
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
    // Open the whole ladder: turn → step/calls aggregate → call result.
    if let ChatBlock::StepGroup { open, steps, .. } = &mut v.blocks[0] {
        *open = true;
        steps[0].open = true;
        steps[0].calls_open = true;
    }
    v.toggle_tool_call_at(0, 3); // walk: [Turn, Step, Calls, Call] → call

    let mut saw_group_row = false;
    let mut saw_step_row = false;
    let mut saw_thinking_header = false;
    let mut copied = Vec::new();
    for line in v.flatten() {
        let text: String = line.spans.iter().map(|s| s.content.clone()).collect();
        match clean_line(&line) {
            None => {
                if text.contains(" Step") && !text.contains("Step(") {
                    saw_group_row = true;
                }
                if text.contains("Step(") {
                    saw_step_row = true;
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
        saw_thinking_header,
        "precondition: the Thinking header was rendered"
    );
    let joined = copied.join("\n");
    assert!(joined.contains("copy me"), "thinking body is copyable");
    assert!(joined.contains("t1-out"), "call output is copyable");
    assert!(
        joined.contains("\u{276f} bash"),
        "the call header row is content, not chrome: {joined:?}"
    );
    assert!(!joined.contains("Step("), "step labels are chrome");
    assert!(!joined.contains("1 Step"), "group rows are chrome");
    assert!(
        !joined.contains("Function call"),
        "calls aggregation row is chrome"
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
    assert_eq!(
        replay_groups[0].len(),
        1,
        "an orphan call without new Thinking stays in the current Step"
    );
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
        flat.iter().any(|l| l.contains("\u{25b8} 2 Steps")),
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

#[test]
fn live_and_replay_share_one_turn_with_the_same_two_steps() {
    let mut live = ChatView::default();
    live.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("go"),
    });
    live.begin_turn();
    live.apply(&SessionEvent::ReasoningDelta("first".into()));
    live.apply(&SessionEvent::TextDelta("checking".into()));
    for id in ["a", "a2"] {
        live.apply(&SessionEvent::ToolStart {
            id: id.into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "echo x"}),
        });
    }
    live.apply(&SessionEvent::ToolEnd {
        id: "a".into(),
        name: "bash".into(),
        output: "A".into(),
        is_error: false,
        images: Vec::new(),
    });
    live.apply(&SessionEvent::ToolEnd {
        id: "a2".into(),
        name: "bash".into(),
        output: "A2".into(),
        is_error: false,
        images: Vec::new(),
    });
    live.apply(&SessionEvent::ReasoningDelta("second".into()));
    live.apply(&SessionEvent::ToolStart {
        id: "b".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    live.apply(&SessionEvent::ToolEnd {
        id: "b".into(),
        name: "bash".into(),
        output: "B".into(),
        is_error: false,
        images: Vec::new(),
    });
    live.apply(&SessionEvent::TextDelta("done".into()));
    live.apply(&SessionEvent::Done);

    let mut user = Message::user("u1", "go");
    user.synthetic = false;
    let (mut a1, t1) = replay_tool_round("a1", "a", Some("first"));
    a1.blocks.push(ContentBlock::text("checking"));
    a1.blocks.push(ContentBlock::ToolUse {
        id: "a2".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    let mut t1 = t1;
    t1.blocks.push(ContentBlock::ToolResult {
        tool_use_id: "a2".into(),
        content: "A2".into(),
        is_error: false,
        images: Vec::new(),
    });
    let (a2, t2) = replay_tool_round("a2", "b", Some("second"));
    let mut say = Message::assistant("a3");
    say.blocks.push(ContentBlock::text("done"));
    let replay = replay_messages("act", &[user, a1, t1, a2, t2, say]);

    let shape = |view: &ChatView| {
        view.blocks
            .iter()
            .filter_map(|block| match block {
                ChatBlock::StepGroup { steps, .. } => Some(
                    steps
                        .iter()
                        .map(|step| {
                            (
                                step.thinking_raw.clone(),
                                step.calls
                                    .iter()
                                    .map(|call| call.id.clone())
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(shape(&live), shape(&replay));
    assert_eq!(shape(&live).len(), 1);
    assert_eq!(shape(&live)[0].len(), 2);
    assert_eq!(shape(&live)[0][0].1, ["a", "a2"]);
}
