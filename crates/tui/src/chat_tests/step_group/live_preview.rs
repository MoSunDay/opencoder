//! Live (streaming) contract: disclosure NEVER moves on its own — every
//! ladder is folded by default and no streaming event opens or closes a
//! level. While the admitted turn runs, the CURRENT (last) step streams in
//! the TURN LIVE REGION — its OWN area between the transcript and the
//! composer (`chat_live_turn.rs`), a DIFFERENT thing from the Say/Step
//! transcript: `flatten` never emits these lines and no ladder block ever
//! contains them. An OPEN Say streams in the same region (marker + raw
//! text) and is NOT transcript content; sealing it (Done/Error/turn
//! boundary) clears the region and lands the Say in the transcript merged
//! into that turn's folded ladder; if the next turn is already running the
//! region simply shows it. An open ladder COEXISTS with the region (its
//! rows plus the streaming panel), and the user's open state survives the
//! settle; Ctrl+L folds levels but the render-only region keeps streaming.
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
        flat_first(&v).starts_with("\u{25b8} 1 Step"),
        "the transcript's ladder row stays folded with its running hint: {:?}",
        flat_first(&v)
    );
    assert_eq!(
        lines(&v).len(),
        2,
        "folded ladder renders exactly its row + blank in the transcript: {:?}",
        lines(&v)
    );
    let lv = live(&v);
    assert!(
        !lv.contains("Step("),
        "live region carries NO step counter row — the ladder owns the counting: {lv}"
    );
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
    assert!(
        lv.contains("1 Function call"),
        "the running call's aggregate row renders (LADDER-IDENTICAL shape): {lv}"
    );
    assert!(
        lv.contains("bash") && lv.contains("echo x"),
        "the call's header row (name + summary) renders like the ladder's: {lv}"
    );
    assert!(
        flat_first(&v).starts_with("\u{25b8} 1 Step"),
        "the transcript's folded group row still carries its own running hint"
    );

    end_call(&mut v, "c1", "done");
    assert_eq!(
        disclosure(&v),
        (false, vec![(false, false)]),
        "call completion is not a Step boundary: the live area keeps streaming"
    );
    assert!(
        live(&v).contains("Function call"),
        "the step's rows stay streaming in the live region after its call ends: {}",
        live(&v)
    );
}

/// Results stay COLLAPSED in the TURN LIVE REGION — `output` is never
/// rendered there, no matter what the ladder does. The ladder (and ONLY
/// the ladder) owns the result drill-down: expanding the call's row in the
/// transcript makes the output appear in `flatten` while the live strip
/// keeps rendering only the ladder-identical aggregate + header rows.
#[test]
fn live_results_stay_collapsed_the_ladder_owns_drilldown() {
    let mut v = ChatView::default();
    reasoning(&mut v, "thinking");
    start_call(&mut v, "c1");
    end_call(&mut v, "c1", "SECRET-OUT-TEXT");
    // Folded ladder: the output is nowhere — not in the transcript, not in
    // the live strip.
    assert!(
        !live(&v).contains("SECRET-OUT-TEXT"),
        "the live region NEVER renders call results: {}",
        live(&v)
    );
    assert!(
        !flat(&v).contains("SECRET-OUT-TEXT"),
        "the folded ladder hides the result too: {}",
        flat(&v)
    );
    // Expand the call through the user toggle walk (group → step → calls
    // aggregate → the call's own row) — the ladder's flatten now carries
    // the output ...
    v.toggle_tool_call_at(0, 0);
    v.toggle_tool_call_at(0, 1);
    v.toggle_tool_call_at(0, 2);
    v.toggle_tool_call_at(0, 3);
    assert!(
        flat(&v).contains("SECRET-OUT-TEXT"),
        "the expanded ladder owns the drill-down: {}",
        flat(&v)
    );
    // ... while the live strip STILL does not — disclosure never leaks in.
    assert!(
        !live(&v).contains("SECRET-OUT-TEXT"),
        "even an expanded ladder never pushes results into the live region: {}",
        live(&v)
    );
    assert!(
        live(&v).contains("bash") && live(&v).contains("echo x"),
        "the strip keeps streaming the ladder-identical header row: {}",
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
        flat_first(&v).starts_with("\u{25b8} 2 Steps"),
        "got {:?}",
        flat_first(&v)
    );
    let lv = live(&v);
    assert!(
        !lv.contains("Step(") && lv.contains("round two"),
        "the live region moves to the new step's rows, counter-less: {lv}"
    );
    assert!(
        lv.contains("round two"),
        "the new step's thinking streams: {lv}"
    );
    assert!(
        !lv.contains("round one") && !lv.contains("Step("),
        "only the CURRENT step streams — the finished one lives inside the \
folded ladder: {lv}"
    );
    assert!(
        !text.contains("round two"),
        "and the transcript stays ladder-only: {text}"
    );
}

#[test]
fn say_streams_in_the_live_region_and_lands_in_the_transcript_on_seal() {
    let mut v = ChatView::default();
    reasoning(&mut v, "think");
    start_call(&mut v, "c1");
    end_call(&mut v, "c1", "out");
    assert!(live(&v).contains("think"));

    // MID-STREAM: the open Say IS transcript content — but the ladder
    // stays FOLDED in its settled counting shape (the merge happens only
    // on seal): the plain `▸ 1 Step` group row, then the standalone
    // `say:` header carrying the spinner, with the raw body rows below it
    // in the transcript. The live region holds NO Say at all (it cleared
    // when the Say began).
    v.apply(&SessionEvent::TextDelta("the answer".into()));
    let text = flat(&v);
    assert!(
        text.contains("\u{25b8} 1 Step"),
        "the group row keeps its plain folded counting shape: {text}"
    );
    assert!(
        text.contains("\u{276f} Say: \u{280b}"),
        "the standalone say: header carries the running spinner: {text}"
    );
    assert!(
        text.contains("the answer"),
        "the open Say's raw body streams in the transcript: {text}"
    );
    let lv = live(&v);
    assert!(
        !lv.contains("\u{1f4ac} Say") && !lv.contains("the answer"),
        "the live region never carries the Say (marker or text): {lv}"
    );
    assert!(
        !lv.contains("think"),
        "the Say disarmed progress — the region shows no step either: {lv}"
    );

    // SEAL: Done finalizes the Say — the pair MERGES here (the folded row
    // disappears into `Say(1 step)`), the raw rows swap for the rendered
    // ones and the spinner hint is gone; the live region stays empty.
    v.apply(&SessionEvent::Done);
    let text = flat(&v);
    assert!(
        text.contains("Say(1 step): the answer") && !text.contains("running"),
        "sealing merges the pair and drops the hint: {text}"
    );
    assert!(
        !text.contains("\u{25b8} 1 Step"),
        "the folded step-count row disappears into the merged header: {text}"
    );
    assert!(
        live_lines(&v).is_empty(),
        "the live region clears — the step lives only inside the folded pair"
    );
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
        lv.contains("more work") && !lv.contains("Step("),
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
    assert!(live(&v).contains("think"));
    v.apply(&SessionEvent::Error("boom".into()));
    assert!(
        live_lines(&v).is_empty(),
        "Error clears the live region like Done"
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

/// TWO SEPARATE AREAS: a user-opened ladder and the live region COEXIST —
/// the TRANSCRIPT holds the ladder's rows, the live region (its own area
/// between transcript and composer) streams the running step; opening the
/// ladder never touches it (no suppression in either direction), and the
/// user's open state survives the settle: streaming never closes a level
/// behind the user's back.
#[test]
fn user_opened_ladder_coexists_with_the_live_region_and_survives_the_settle() {
    let mut v = ChatView::default();
    reasoning(&mut v, "round one");
    start_call(&mut v, "c1");
    v.toggle_tool_call_at(0, 0); // open the ladder mid-run
    let text = flat(&v);
    let lv = live(&v);
    assert!(
        text.contains("Step(1)") && !text.contains("round one"),
        "the TRANSCRIPT holds the ladder only — step rows, thinking folded \
(the step itself is closed): {text}"
    );
    assert!(
        !lv.contains("Step(") && lv.contains("\u{1f4ad} Thinking") && lv.contains("round one"),
        "the LIVE REGION streams the same running step forced open — a \
different area, untouched by the ladder's disclosure: {lv}"
    );

    reasoning(&mut v, " round two");
    assert_eq!(
        disclosure(&v),
        (true, vec![(false, false), (false, false)]),
        "the new running step still opens nothing inside the user-open ladder"
    );
    assert!(
        live(&v).ends_with("round two"),
        "the live region now streams the NEW current step: {}",
        live(&v)
    );
    v.apply(&SessionEvent::TextDelta("answer".into()));
    assert_eq!(
        disclosure(&v),
        (true, vec![(false, false), (false, false)]),
        "settle never closes a level the user opened"
    );
    let lv = live(&v);
    let text = flat(&v);
    assert!(
        !lv.contains("\u{1f4ac} Say") && !lv.contains("answer"),
        "the Say began — the live region disarmed and carries no Say rows: {lv}"
    );
    assert!(
        text.contains("\u{276f} Say: \u{280b}") && text.contains("answer"),
        "the open Say streams standalone (say: header + spinner + raw body) \
below the user-open ladder — the merge waits for seal: {text}"
    );
    v.apply(&SessionEvent::Done);
    assert!(
        live_lines(&v).is_empty(),
        "and clears the live region like every settle"
    );
    assert!(
        flat(&v).contains("Say(2 steps): answer"),
        "sealing merges the pair into the user-open ladder's header"
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
    let lv = live(&v);
    assert!(
        lv.contains("thinking more") && !lv.contains("Step("),
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
