//! Live (streaming) contract: disclosure NEVER moves on its own — every
//! ladder is folded by default and no streaming event opens or closes a
//! level. While the admitted turn runs (`progress_active`, i.e. no Say
//! yet), the group is ABSENT from the transcript (zero rows, zero hit
//! targets) and the CURRENT (last) step streams in the TURN LIVE REGION —
//! its OWN area between the transcript and the composer
//! (`chat_live_turn.rs`), the running turn's ONLY surface. Say's first
//! chunk, run end, Error, or a new turn settles the group: it re-enters
//! the transcript as a plain folded ladder (merged into the Say header
//! when adjacent), the region clears, and if the next turn is already
//! running the region simply shows it. There is no ladder to open while
//! the turn runs; after the settle the user's open state is ordinary
//! ladder state and survives rerenders; Ctrl+L folds levels but the
//! render-only region keeps streaming. Replay/snapshot ladders never
//! stream (`replay.rs`).

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

/// The turn live region's rows joined (`live_lines`, see chat_tests/mod.rs).
fn live(view: &ChatView) -> String {
    live_lines(view).join("\n")
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
        lines(&v).is_empty(),
        "the transcript carries NO row for the running group — not even the \
folded ladder header: {:?}",
        lines(&v)
    );
    let lv = live(&v);
    assert!(lv.contains("Step(1)"), "live region step row: {lv}");
    assert!(
        lv.contains("\u{1f4ad} Thinking"),
        "live region thinking header: {lv}"
    );
    assert!(
        lv.contains("first thought"),
        "live thinking streams in the live region — no click needed: {lv}"
    );
    assert!(
        !text.contains("first thought"),
        "the TRANSCRIPT never carries the live region's rows: {text}"
    );

    reasoning(&mut v, " grows");
    assert!(
        live(&v).contains("first thought grows"),
        "further deltas of the SAME reasoning run keep streaming: {}",
        live(&v)
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
    let lv = live(&v);
    assert!(lv.contains("1 Function call"), "calls aggregate: {lv}");
    assert!(lv.contains("bash"), "running call row renders: {lv}");
    assert!(
        lines(&v).is_empty(),
        "the transcript carries no folded group row while the turn runs: {:?}",
        lines(&v)
    );

    end_call(&mut v, "c1", "done");
    assert_eq!(
        disclosure(&v),
        (false, vec![(false, false)]),
        "call completion is not a Step boundary: the live area keeps streaming"
    );
    assert!(
        live(&v).contains("Step(1)"),
        "the step stays visible in the live region after its call ends: {}",
        live(&v)
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
        lines(&v).is_empty(),
        "while the turn runs the transcript shows nothing — not even the \
folded `2 Steps` row: {:?}",
        lines(&v)
    );
    let lv = live(&v);
    assert!(
        lv.contains("Step(2)"),
        "the live region moves to the new step: {lv}"
    );
    assert!(
        lv.contains("round two"),
        "the new step's thinking streams: {lv}"
    );
    assert!(
        !lv.contains("round one") && !lv.contains("Step(1)"),
        "only the CURRENT step streams — the finished one waits inside the \
group until the turn settles: {lv}"
    );
    assert!(
        !text.contains("round two"),
        "and the transcript stays empty while the turn runs: {text}"
    );
}

#[test]
fn say_first_chunk_and_run_end_clear_the_live_area() {
    let mut v = ChatView::default();
    reasoning(&mut v, "think");
    start_call(&mut v, "c1");
    end_call(&mut v, "c1", "out");
    assert!(live(&v).contains("Step(1)"));

    v.apply(&SessionEvent::TextDelta("the answer".into()));
    assert_eq!(
        lines(&v),
        vec!["\u{25b8} Say(1 step): the answer  \u{280b} running ", ""],
        "on Say the settled group re-enters the transcript folded, merged \
into the Say header (single-line body hidden, spinner on the header)"
    );
    assert!(
        live_lines(&v).is_empty(),
        "the live region clears — the step lives only inside the folded pair"
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
    let lv = live(&v);
    assert!(
        lv.contains("more work") && lv.contains("Step(1)"),
        "the second ladder's running step streams in the live region: {lv}"
    );
    v.apply(&SessionEvent::Done);
    assert!(
        live_lines(&v).is_empty(),
        "Done settles the second ladder as well"
    );
}

#[test]
fn error_clears_the_live_area_like_done() {
    let mut v = ChatView::default();
    reasoning(&mut v, "think");
    assert!(live(&v).contains("Step(1)"));
    v.apply(&SessionEvent::Error("boom".into()));
    assert!(
        live_lines(&v).is_empty(),
        "Error clears the live region like Done"
    );
    assert_eq!(
        lines(&v),
        vec!["\u{25b8} 1 Step", "", "error: boom"],
        "Error settles the group: it re-enters the transcript as a plain \
folded ladder (no running hint), above the error marker"
    );
}

/// The live area is render-only: it must not create click targets. While
/// the turn runs the group is absent from the transcript, so there are NO
/// targets at all — not even the group row — and even a programmatically
/// opened ladder renders nothing until the turn settles.
#[test]
fn live_area_rows_are_not_clickable_targets() {
    let mut v = ChatView::default();
    reasoning(&mut v, "thinking");
    start_call(&mut v, "c1");
    let headers = v.tool_call_headers();
    assert!(
        headers.is_empty(),
        "a running group has no transcript rows, so no targets either: {headers:?}"
    );
    // Toggling the hidden group's state changes nothing on screen: no rows
    // appear while the turn runs; the live area keeps streaming regardless.
    v.toggle_tool_call_at(0, 0);
    assert!(
        v.tool_call_headers().is_empty() && lines(&v).is_empty(),
        "an open-but-running ladder still renders no transcript rows: {:?}",
        lines(&v)
    );
    assert!(
        live(&v).contains("Step(1)"),
        "the live area keeps streaming: {}",
        live(&v)
    );
}

/// TWO SEPARATE SURFACES: while the turn runs there is NO ladder to open —
/// the group is absent from the transcript (no rows, no hit target) and
/// the LIVE REGION (its own area between transcript and composer) is the
/// turn's only surface. After the settle the folded ladder enters the
/// transcript (merged into the Say header when adjacent); the user opens
/// it, and that disclosure state survives rerenders — streaming never
/// closes a level behind the user's back.
#[test]
fn no_ladder_to_open_while_running_settled_ladder_opens_and_survives() {
    let mut v = ChatView::default();
    reasoning(&mut v, "round one");
    start_call(&mut v, "c1");
    assert!(
        lines(&v).is_empty() && v.tool_call_headers().is_empty(),
        "while running: no transcript rows, nothing to click: {:?}",
        lines(&v)
    );
    assert!(
        live(&v).contains("Step(1)") && live(&v).contains("\u{1f4ad} Thinking"),
        "the LIVE REGION streams the running step forced open: {}",
        live(&v)
    );

    reasoning(&mut v, " round two");
    assert_eq!(
        disclosure(&v),
        (false, vec![(false, false), (false, false)]),
        "the new running step still opens nothing"
    );
    assert!(
        live(&v).ends_with("round two"),
        "the live region now streams the NEW current step: {}",
        live(&v)
    );
    assert!(
        lines(&v).is_empty(),
        "and the transcript still shows nothing while the turn runs"
    );

    v.apply(&SessionEvent::TextDelta("answer".into()));
    assert!(
        live_lines(&v).is_empty(),
        "the settle clears the live region like every settle"
    );
    let text = flat(&v);
    assert!(
        text.contains("Say(2 steps): answer"),
        "the settled group re-enters the transcript folded, merged into the \
Say header: {text}"
    );

    // NOW there is a ladder to open; its disclosure survives rerenders.
    v.toggle_tool_call_at(0, 0);
    assert_eq!(
        disclosure(&v),
        (true, vec![(false, false), (false, false)]),
        "the user's open survives the settle boundary"
    );
    let opened = flat(&v);
    assert!(
        opened.contains("Step(1)") && opened.contains("Step(2)"),
        "the open ladder lists both step rows: {opened}"
    );
    assert_eq!(
        flat(&v),
        opened,
        "disclosure state survives rerenders unchanged"
    );
}

/// Ctrl+L folds every collapsible level, but the live area is not a
/// disclosure level — the running step keeps streaming while the turn runs.
#[test]
fn collapse_all_keeps_the_live_region_visible() {
    let mut v = ChatView::default();
    reasoning(&mut v, "thinking");
    v.collapse_all_collapsible();
    assert_eq!(disclosure(&v), (false, vec![(false, false)]));
    reasoning(&mut v, " more");
    start_call(&mut v, "c1");
    assert!(
        lines(&v).is_empty(),
        "collapse_all during the run leaves no ladder rows — there were none: {:?}",
        lines(&v)
    );
    let lv = live(&v);
    assert!(
        lv.contains("thinking more") && lv.contains("Step(1)"),
        "the render-only live region survives Ctrl+L: {lv}"
    );
    v.apply(&SessionEvent::Done);
    assert!(live_lines(&v).is_empty(), "and clears normally at run end");
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
