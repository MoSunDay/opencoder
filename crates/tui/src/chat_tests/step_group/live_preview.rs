//! Live (streaming) contract: disclosure NEVER moves on its own — every
//! ladder is folded by default and no streaming event opens or closes a
//! level. While the admitted turn runs (`progress_active`, i.e. no Say
//! yet), the CURRENT (last) step streams in the TURN LIVE AREA — an
//! independent render-only region at the transcript tail
//! (`chat_live_turn.rs`), never inside a ladder block and never coupled to
//! any ladder's disclosure state. Say's first chunk, run end, Error, or a
//! new turn retires the area, and the step then lives only inside that
//! turn's folded ladder; if the next turn is already running the area
//! simply shows it. An open ladder COEXISTS with the live area (its rows
//! plus the streaming tail), and the user's open state survives the
//! settle; Ctrl+L folds levels but the render-only area keeps streaming.
//! Replay/snapshot ladders never stream (`replay.rs`).

use super::*;

/// (group_open, [(step_open, calls_open)]) for the FIRST group in the flow.
fn disclosure(view: &ChatView) -> (bool, Vec<(bool, bool)>) {
    view.blocks
        .iter()
        .find_map(|block| match block {
            ChatBlock::StepGroup { open, steps, .. } => Some((
                *open,
                steps.iter().map(|s| (s.open, s.calls_open)).collect(),
            )),
            _ => None,
        })
        .expect("expected a step group")
}

fn flat(view: &ChatView) -> String {
    lines(view).join("\n")
}

fn flat_first(view: &ChatView) -> String {
    lines(view).first().cloned().unwrap_or_default()
}

fn reasoning(view: &mut ChatView, text: &str) {
    view.apply(&SessionEvent::ReasoningDelta(text.into()));
}

fn start_call(view: &mut ChatView, id: &str) {
    view.apply(&SessionEvent::ToolStart {
        id: id.into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
}

fn end_call(view: &mut ChatView, id: &str, output: &str) {
    view.apply(&SessionEvent::ToolEnd {
        id: id.into(),
        name: "bash".into(),
        output: output.into(),
        is_error: false,
        images: Vec::new(),
    });
}

#[test]
fn running_reasoning_streams_in_the_turn_live_area() {
    let mut v = ChatView::default();
    reasoning(&mut v, "first thought");
    assert_eq!(
        disclosure(&v),
        (false, vec![(false, false)]),
        "the ladder itself stays folded — the live area is render-only"
    );
    let text = flat(&v);
    assert!(
        flat_first(&v).starts_with("\u{25b8} 1 Step"),
        "folded group row with its running hint: {:?}",
        flat_first(&v)
    );
    assert!(text.contains("Step(1)"), "live area step row: {text}");
    assert!(
        text.contains("\u{1f4ad} Thinking"),
        "live area thinking header: {text}"
    );
    assert!(
        text.contains("first thought"),
        "live thinking streams in the live area — no click needed: {text}"
    );

    reasoning(&mut v, " grows");
    assert!(
        flat(&v).contains("first thought grows"),
        "further deltas of the SAME reasoning run keep streaming: {}",
        flat(&v)
    );
}

#[test]
fn running_calls_stream_with_spinner_and_the_ladder_stays_folded() {
    let mut v = ChatView::default();
    reasoning(&mut v, "thinking");
    start_call(&mut v, "c1");
    assert_eq!(
        disclosure(&v),
        (false, vec![(false, false)]),
        "a joining call opens nothing — the live area renders it instead"
    );
    let text = flat(&v);
    assert!(text.contains("1 Function call"), "calls aggregate: {text}");
    assert!(text.contains("bash"), "running call row renders: {text}");
    assert!(
        text.contains("\u{25b8} 1 Step"),
        "the folded group row still carries its own running hint: {text}"
    );

    end_call(&mut v, "c1", "done");
    assert_eq!(
        disclosure(&v),
        (false, vec![(false, false)]),
        "call completion is not a Step boundary: the live area keeps streaming"
    );
    assert!(
        flat(&v).contains("Step(1)"),
        "the step stays visible in the live area after its call ends: {}",
        flat(&v)
    );
}

#[test]
fn a_new_step_moves_the_live_area_and_folds_the_previous_one() {
    let mut v = ChatView::default();
    reasoning(&mut v, "round one");
    start_call(&mut v, "c1");
    end_call(&mut v, "c1", "out");
    reasoning(&mut v, "round two");
    assert_eq!(
        disclosure(&v),
        (false, vec![(false, false), (false, false)]),
        "a new step opens nothing; the ladder stays fully folded"
    );
    let text = flat(&v);
    assert!(
        flat_first(&v).starts_with("\u{25b8} 2 Steps"),
        "got {:?}",
        flat_first(&v)
    );
    assert!(
        text.contains("Step(2)"),
        "the live area moves to the new step: {text}"
    );
    assert!(
        text.contains("round two"),
        "the new step's thinking streams: {text}"
    );
    assert!(
        !text.contains("round one") && !text.contains("Step(1)"),
        "only the CURRENT step streams — the finished one lives inside the \
folded ladder: {text}"
    );
}

#[test]
fn say_first_chunk_and_run_end_clear_the_live_area() {
    let mut v = ChatView::default();
    reasoning(&mut v, "think");
    start_call(&mut v, "c1");
    end_call(&mut v, "c1", "out");
    assert!(flat(&v).contains("Step(1)"));

    v.apply(&SessionEvent::TextDelta("the answer".into()));
    let text = flat(&v);
    assert!(
        text.contains("Say(1 step): the answer"),
        "Say's first chunk settles the pair into the merged header: {text}"
    );
    assert!(
        !text.contains("Step(1)") && !text.contains("think"),
        "the live area retires — the step lives only inside the folded pair: {text}"
    );

    v.apply(&SessionEvent::Done);
    assert_eq!(
        disclosure(&v),
        (false, vec![(false, false)]),
        "Done changes nothing disclosure-wise (the live area was already gone)"
    );

    // A post-Say ladder re-arms: new activity below the Say streams fresh
    // in the live area, on the NEW turn only.
    reasoning(&mut v, "more work");
    start_call(&mut v, "c2");
    assert!(!disclosure(&v).0, "the closed ladder above stays closed");
    let text = flat(&v);
    assert!(
        text.contains("more work") && text.contains("Step(1)"),
        "the second ladder's running step streams in the live area: {text}"
    );
    v.apply(&SessionEvent::Done);
    assert!(
        !flat(&v).contains("more work"),
        "Done settles the second ladder as well: {}",
        flat(&v)
    );
}

#[test]
fn error_clears_the_live_area_like_done() {
    let mut v = ChatView::default();
    reasoning(&mut v, "think");
    assert!(flat(&v).contains("Step(1)"));
    v.apply(&SessionEvent::Error("boom".into()));
    let text = flat(&v);
    assert!(
        !text.contains("Step(1)") && !text.contains("think"),
        "Error retires the live area like Done: {text}"
    );
}

/// The live area is render-only: it must not create click targets (the
/// folded ladder still yields exactly its group row) — `visible_targets`
/// and the mouse map stay unchanged while it streams.
#[test]
fn live_area_rows_are_not_clickable_targets() {
    let mut v = ChatView::default();
    reasoning(&mut v, "thinking");
    start_call(&mut v, "c1");
    let headers = v.tool_call_headers();
    assert_eq!(
        headers.len(),
        1,
        "only the group row is clickable while the live area streams: {headers:?}"
    );
    // Clicking the group row opens the ladder — plain toggle, no pins; the
    // live area at the tail keeps streaming regardless.
    v.toggle_tool_call_at(0, 0);
    let headers = v.tool_call_headers();
    assert_eq!(
        headers.len(),
        2,
        "group + step row (calls aggregate waits for step.open): {headers:?}"
    );
}

/// TWO SEPARATE REGIONS: a user-opened ladder and the live area COEXIST —
/// opening the ladder never touches the streaming tail (no suppression in
/// either direction), and the user's open state survives the settle:
/// streaming never closes a level behind the user's back.
#[test]
fn user_opened_ladder_coexists_with_the_live_area_and_survives_the_settle() {
    let mut v = ChatView::default();
    reasoning(&mut v, "round one");
    start_call(&mut v, "c1");
    v.toggle_tool_call_at(0, 0); // open the ladder mid-run
    let text = flat(&v);
    assert!(
        text.contains("Step(1)") && text.contains("round one"),
        "open ladder renders the step ROW in-region while the live area at \
the tail streams the same running step's thinking — coexistence: {text}"
    );
    let live_tail = &text[text.rfind("Step(1)").unwrap()..];
    assert!(
        live_tail.contains("💭 Thinking") && live_tail.contains("round one"),
        "the live area (forced open) trails every block, untouched by the \
ladder's disclosure: {text}"
    );

    reasoning(&mut v, " round two");
    assert_eq!(
        disclosure(&v),
        (true, vec![(false, false), (false, false)]),
        "the new running step still opens nothing inside the user-open ladder"
    );
    let text = flat(&v);
    assert!(
        text.ends_with("round two\n"),
        "the live area now streams the NEW current step: {text}"
    );

    v.apply(&SessionEvent::TextDelta("answer".into()));
    assert_eq!(
        disclosure(&v),
        (true, vec![(false, false), (false, false)]),
        "settle never closes a level the user opened"
    );
    assert!(
        !flat(&v).contains("round two"),
        "and retires the live area like every settle: {}",
        flat(&v)
    );
}

/// Ctrl+L folds every collapsible level, but the live area is not a
/// disclosure level — the running step keeps streaming while the turn runs.
#[test]
fn collapse_all_keeps_the_live_area_visible() {
    let mut v = ChatView::default();
    reasoning(&mut v, "thinking");
    v.collapse_all_collapsible();
    assert_eq!(disclosure(&v), (false, vec![(false, false)]));
    reasoning(&mut v, " more");
    start_call(&mut v, "c1");
    let text = flat(&v);
    assert!(
        text.contains("thinking more") && text.contains("Step(1)"),
        "the render-only live area survives Ctrl+L: {text}"
    );
    v.apply(&SessionEvent::Done);
    assert!(
        !flat(&v).contains("Step(1)"),
        "and retires normally at run end: {}",
        flat(&v)
    );
}

#[test]
fn replay_ladders_carry_no_live_area() {
    // Mirrors `replay.rs::replay_tool_round`: a reasoning+tool-use assistant
    // message followed by its tool result. Replayed ladders are at rest —
    // fully folded, no live area.
    let mut asst = Message::assistant("a1");
    asst.blocks.push(ContentBlock::Reasoning {
        text: "pondering".into(),
    });
    asst.blocks.push(ContentBlock::ToolUse {
        id: "t1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "ls"}),
    });
    let mut tool_msg = Message::assistant("t1-res");
    tool_msg.role = Role::Tool;
    tool_msg.blocks = vec![ContentBlock::ToolResult {
        tool_use_id: "t1".into(),
        content: "file".into(),
        is_error: false,
        images: Vec::new(),
    }];
    let v = replay_messages("build", &[asst, tool_msg]);
    assert_eq!(
        disclosure(&v),
        (false, vec![(false, false)]),
        "replayed ladders never stream, even though the steps are complete"
    );
    let text = flat(&v);
    assert!(
        !text.contains("Step(1)") && !text.contains("pondering"),
        "no live-area rows leak into a replayed transcript: {text}"
    );
}
