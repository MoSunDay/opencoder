//! Live (streaming) disclosure contract: while the admitted turn runs, the
//! ladder auto-opens at its running position so thinking deltas and call
//! rows stream visibly; the position retires (Say's first chunk, run end,
//! new turn) back into the folded `N Steps` summary. User clicks always
//! win: a touched level is pinned (`user_touched`) and the automatic
//! open/settle cycle never moves it again for the rest of the turn.
//! Replay/snapshot ladders never auto-open (see `replay.rs`).

use super::*;

/// (group_open, [(step_open, calls_open)]) for the FIRST group in the flow.
fn disclosure(view: &ChatView) -> (bool, Vec<(bool, bool)>) {
    view.blocks
        .iter()
        .find_map(|block| match block {
            ChatBlock::StepGroup { open, steps, .. } => Some((
                *open,
                steps
                    .iter()
                    .map(|s| (s.open, s.calls_open))
                    .collect(),
            )),
            _ => None,
        })
        .expect("expected a step group")
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
fn live_reasoning_auto_opens_the_running_position() {
    let mut v = ChatView::default();
    reasoning(&mut v, "first thought");
    assert_eq!(
        disclosure(&v),
        (true, vec![(true, true)]),
        "the first reasoning delta opens the group row AND the running step"
    );
    // The streamed thinking renders live — no click needed.
    let flat = lines(&v).join("\n");
    assert!(flat.contains("first thought"), "live thinking is visible: {flat:?}");

    reasoning(&mut v, " grows");
    assert_eq!(
        disclosure(&v),
        (true, vec![(true, true)]),
        "further deltas of the SAME reasoning run keep the position open"
    );
}

#[test]
fn tool_activity_keeps_the_running_position_open() {
    let mut v = ChatView::default();
    reasoning(&mut v, "thinking");
    start_call(&mut v, "c1");
    assert_eq!(
        disclosure(&v),
        (true, vec![(true, true)]),
        "a call joins the open running step (calls aggregate auto-open)"
    );
    // The running call row carries the motion glyph.
    let flat = lines(&v).join("\n");
    assert!(flat.contains("bash"), "call row renders while running: {flat:?}");
    end_call(&mut v, "c1", "done");
    assert_eq!(
        disclosure(&v),
        (true, vec![(true, true)]),
        "call completion is not a Step boundary: the position stays open"
    );
}

#[test]
fn a_new_step_folds_the_finished_one_and_takes_over() {
    let mut v = ChatView::default();
    reasoning(&mut v, "round one");
    start_call(&mut v, "c1");
    end_call(&mut v, "c1", "out");
    reasoning(&mut v, "round two");
    assert_eq!(
        disclosure(&v),
        (true, vec![(false, false), (true, true)]),
        "the finished step collapses; the fresh running step takes the open slot"
    );
    // The group row stays open (with its live running hint) the whole time.
    assert!(flat_first(&v).starts_with("\u{276f} 2 Steps"), "got {:?}", flat_first(&v));
}

/// LIVE FOCUS: the auto-managed running position renders ONLY the running
/// (last) step — thinking + its calls — so a turn with many tool rounds does
/// not flood the viewport with one `Step(i)` row each. The group row keeps
/// the total count; a user click opens the full ladder (user priority); Say
/// settles the auto position back into the collapsed `Say(n steps)` pair.
#[test]
fn live_running_position_renders_only_the_last_step() {
    let mut v = ChatView::default();
    reasoning(&mut v, "round one");
    start_call(&mut v, "c1");
    end_call(&mut v, "c1", "out-a");
    reasoning(&mut v, "round two");
    start_call(&mut v, "c2");
    end_call(&mut v, "c2", "out-b");
    reasoning(&mut v, "round three");

    let flat = lines(&v).join("\n");
    assert!(flat.contains("3 Steps"), "group row keeps the count: {flat}");
    assert!(
        flat.contains("round three"),
        "running thinking streams in place: {flat}"
    );
    assert!(flat.contains("Step(3)"), "running step row renders: {flat}");
    assert!(!flat.contains("Step(1)"), "finished steps stay folded: {flat}");
    assert!(!flat.contains("Step(2)"), "finished steps stay folded: {flat}");
    assert!(!flat.contains("round one"), "old thinking stays hidden: {flat}");
    assert!(!flat.contains("out-a"), "finished call output hidden: {flat}");

    // Header accounting mirrors the hidden rows: only the group row and the
    // running step row are clickable, exactly one line apart.
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 2, "group + running step only: {headers:?}");
    assert_eq!(headers[1].header_line_idx, headers[0].header_line_idx + 1);

    // User opens the turn row: every step row comes back (user priority).
    v.toggle_tool_call_at(0, 0); // fold (pins)
    v.toggle_tool_call_at(0, 0); // reopen (pinned)
    let flat = lines(&v).join("\n");
    assert!(
        flat.contains("Step(1)") && flat.contains("Step(2)") && flat.contains("Step(3)"),
        "user-opened ladder renders every step row: {flat}"
    );
}

/// Say settles the auto-managed running position back into the collapsed
/// `Say(n steps)` pair — the pre-live-focus summary the TUI started from.
#[test]
fn live_settle_folds_the_running_step_back_into_the_say_pair() {
    let mut v = ChatView::default();
    reasoning(&mut v, "round one");
    start_call(&mut v, "c1");
    end_call(&mut v, "c1", "out");
    reasoning(&mut v, "round two");
    v.apply(&SessionEvent::TextDelta("the answer".into()));

    let flat = lines(&v).join("\n");
    assert!(flat.contains("Say(2 steps)"), "folded pair header: {flat}");
    assert!(
        !flat.contains("Step("),
        "settled ladder hides every step row: {flat}"
    );
}

fn flat_first(v: &ChatView) -> String {
    lines(v).first().cloned().unwrap_or_default()
}

#[test]
fn say_first_chunk_and_run_end_settle_the_ladder() {
    let mut v = ChatView::default();
    reasoning(&mut v, "think");
    start_call(&mut v, "c1");
    end_call(&mut v, "c1", "out");
    assert!(disclosure(&v).0);

    v.apply(&SessionEvent::TextDelta("the answer".into()));
    assert_eq!(
        disclosure(&v),
        (false, vec![(false, false)]),
        "Say's first chunk retires the auto-opened position into the folded summary"
    );

    // A post-Say ladder re-arms: new activity below the Say opens fresh.
    reasoning(&mut v, "more work");
    start_call(&mut v, "c2");
    assert!(!disclosure(&v).0, "the closed ladder above stays closed");
    let group_two = v
        .blocks
        .iter()
        .rev()
        .find_map(|b| match b {
            ChatBlock::StepGroup { open, steps, .. } => Some((
                *open,
                steps.iter().map(|s| (s.open, s.calls_open)).collect::<Vec<_>>(),
            )),
            _ => None,
        })
        .expect("second ladder");
    assert_eq!(group_two, (true, vec![(true, true)]));

    v.apply(&SessionEvent::TextDelta("second answer".into()));
    v.apply(&SessionEvent::Done);
    assert_eq!(
        group_two_of(&v),
        (false, vec![(false, false)]),
        "Done settles the second ladder as well"
    );
}

fn group_two_of(v: &ChatView) -> (bool, Vec<(bool, bool)>) {
    v.blocks
        .iter()
        .rev()
        .find_map(|b| match b {
            ChatBlock::StepGroup { open, steps, .. } => Some((
                *open,
                steps.iter().map(|s| (s.open, s.calls_open)).collect::<Vec<_>>(),
            )),
            _ => None,
        })
        .expect("ladder")
}

#[test]
fn error_settles_the_ladder_like_done() {
    let mut v = ChatView::default();
    reasoning(&mut v, "think");
    v.apply(&SessionEvent::Error("boom".into()));
    assert_eq!(
        disclosure(&v),
        (false, vec![(false, false)]),
        "Error retires the auto-opened position"
    );
}

#[test]
fn user_collapse_of_an_auto_opened_level_wins_for_the_turn() {
    let mut v = ChatView::default();
    reasoning(&mut v, "thinking");
    assert_eq!(disclosure(&v), (true, vec![(true, true)]));

    // The user folds the group row while the turn is still running.
    v.toggle_tool_call_at(0, 0);
    assert_eq!(disclosure(&v), (false, vec![(true, true)]));

    reasoning(&mut v, " more");
    start_call(&mut v, "c1");
    assert_eq!(
        disclosure(&v),
        (false, vec![(true, true)]),
        "streaming must not reopen a level the user closed"
    );

    // The user also folds the step row: the auto logic stays off it. A
    // step-row click flips only `open` — the calls aggregate flag rides
    // along untouched (pre-existing toggle semantics).
    v.toggle_tool_call_at(0, 0); // reopen group to reach the step row
    v.toggle_tool_call_at(0, 1); // fold Step(1)
    assert_eq!(disclosure(&v), (true, vec![(false, true)]));
    reasoning(&mut v, " still running");
    assert_eq!(
        disclosure(&v),
        (true, vec![(false, true), (true, true)]),
        "the user-folded step never reopens; the reasoning run AFTER the \
call births a fresh step, which takes over as the running position"
    );
}

#[test]
fn collapse_all_pins_levels_against_the_stream() {
    let mut v = ChatView::default();
    reasoning(&mut v, "thinking");
    v.collapse_all_collapsible();
    assert_eq!(disclosure(&v), (false, vec![(false, false)]));
    reasoning(&mut v, " more");
    start_call(&mut v, "c1");
    assert_eq!(
        disclosure(&v),
        (false, vec![(false, false)]),
        "Ctrl+L suppresses the auto-disclosure until the turn ends"
    );
    // A NEW step still folds nothing (already folded) but never reopens.
    reasoning(&mut v, "round two");
    assert_eq!(
        disclosure(&v),
        (false, vec![(false, false), (true, true)]),
        "pinned levels stay folded while the turn runs; the fresh step is a \
new level the policy opens — invisible behind the pinned-closed group row"
    );
}

#[test]
fn user_opened_levels_survive_the_settle() {
    let mut v = ChatView::default();
    reasoning(&mut v, "round one");
    start_call(&mut v, "c1");
    end_call(&mut v, "c1", "out");
    // The step row was auto-opened; toggling it twice (close, reopen) pins
    // it as USER-opened while leaving the exact same visible state.
    v.toggle_tool_call_at(0, 1); // Step(1) closed (pinned)
    v.toggle_tool_call_at(0, 1); // Step(1) reopened (pinned)
    reasoning(&mut v, "round two");
    assert_eq!(
        disclosure(&v),
        (true, vec![(true, true), (true, true)]),
        "a step the user pinned open keeps its disclosure when the next step takes over"
    );
    v.apply(&SessionEvent::TextDelta("answer".into()));
    assert_eq!(
        disclosure(&v),
        (false, vec![(true, true), (false, false)]),
        "settle folds only the auto-managed levels; the pinned step stays open behind the group row"
    );
}

#[test]
fn replay_ladders_stay_fully_collapsed() {
    // Mirrors `replay.rs::replay_tool_round`: a reasoning+tool-use assistant
    // message followed by its tool result. Replay never passes through the
    // live disclosure hooks, so the ladder renders fully collapsed.
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
        "replayed ladders never auto-open, even though the steps are complete"
    );
}
