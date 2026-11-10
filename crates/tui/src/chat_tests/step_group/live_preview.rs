//! Live (streaming) contract: disclosure NEVER moves on its own — every
//! ladder is folded by default and no streaming event opens or closes a
//! level. While the admitted turn runs, the CURRENT (last) step streams in
//! the TURN LIVE REGION — INLINE rows emitted by `flatten` directly BELOW
//! the running group's ladder and ABOVE any later blocks, inside the same
//! scroll window (`chat_live_turn.rs`). The LADDER renders SETTLED history
//! only (`steps::settled_steps`): while the FIRST step is still in flight
//! the block emits ZERO rows and the region is the turn's ONLY surface.
//! The region opens with a `> streaming` header row (collapsed: `▸ streaming`, via
//! `live_collapsed` / `toggle_live_region` / Ctrl+L) — its ONLY step
//! counter-free rows; no ladder block ever contains the streaming step. An
//! OPEN Say streams as TRANSCRIPT CONTENT instead (the region clears when
//! the Say begins): the standalone `say:` header + raw rows, sealing
//! (Done/Error/turn boundary) merges the Say into that turn's folded
//! ladder; if the next turn is already running the region simply shows it.
//! An open ladder COEXISTS with the region (its rows plus the streaming
//! panel), and the user's open state survives the settle. Replay/snapshot
//! ladders never stream (`replay.rs`).

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
    // While the FIRST step streams the ladder holds ZERO settled steps, so
    // the group block emits NO rows at all: the region's rows are the
    // turn's ONLY surface, INLINE at the top of the flatten output.
    assert_eq!(
        lines(&v),
        vec![
            "> streaming".to_string(),
            "    \u{1f4ad} Thinking".to_string(),
            "        first thought".to_string(),
        ],
        "first in-flight step: zero ladder rows, the live region is the \
whole turn surface: {:?}",
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
        flat(&v).contains("first thought"),
        "the region rides INLINE inside the transcript window: {}",
        flat(&v)
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
        flat_first(&v) == "> streaming",
        "still the FIRST step — zero ladder rows, the region's header leads \
the transcript: {:?}",
        flat_first(&v)
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
/// transcript makes the output appear in `flatten` while the live region
/// keeps rendering only the CURRENT step's aggregate + header rows.
#[test]
fn live_results_stay_collapsed_the_ladder_owns_drilldown() {
    let mut v = ChatView::default();
    reasoning(&mut v, "thinking");
    start_call(&mut v, "c1");
    end_call(&mut v, "c1", "SECRET-OUT-TEXT");
    reasoning(&mut v, "current step");
    // Folded ladder: the output is nowhere — not in the transcript ladder,
    // not in the live region (which streams the CURRENT step only).
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
    // Expand the call through the user toggle walk on the SETTLED ladder
    // (group → step → calls aggregate → the call's own row) — the ladder's
    // flatten now carries the output ...
    v.toggle_tool_call_at(0, 0);
    v.toggle_tool_call_at(0, 1);
    v.toggle_tool_call_at(0, 2);
    v.toggle_tool_call_at(0, 3);
    assert!(
        flat(&v).contains("SECRET-OUT-TEXT"),
        "the expanded ladder owns the drill-down: {}",
        flat(&v)
    );
    // ... while the live region STILL does not — disclosure never leaks in.
    assert!(
        !live(&v).contains("SECRET-OUT-TEXT"),
        "even an expanded ladder never pushes results into the live region: {}",
        live(&v)
    );
    assert!(
        live(&v).contains("current step"),
        "the region keeps streaming the CURRENT step's rows: {}",
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
    assert!(
        flat_first(&v).starts_with("\u{25b8} 1 Step"),
        "the ladder row counts SETTLED steps only (the second streams in the \
region): {:?}",
        flat_first(&v)
    );
    let rows = lines(&v);
    assert_eq!(
        rows.len(),
        5,
        "settled group row + blank, then the INLINE region rows: {rows:?}"
    );
    assert_eq!(
        rows[2], "> streaming",
        "the region's header sits directly below the ladder: {rows:?}"
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
        rows.join("\n").contains("round two"),
        "the region's rows ride INLINE in the transcript window: {rows:?}"
    );
}

#[test]
fn say_streams_in_the_live_region_and_lands_in_the_transcript_on_seal() {
    let mut v = ChatView::default();
    reasoning(&mut v, "think");
    start_call(&mut v, "c1");
    end_call(&mut v, "c1", "out");
    assert!(live(&v).contains("think"));

    // MID-STREAM: the open Say IS transcript content — and the ladder has
    // RETIRED (no `▸ 1 Step` counting row above the answer; the count
    // returns on seal): the standalone `say:` header carries the spinner,
    // with the raw body rows below it in the transcript. The live region
    // holds NO Say at all (it cleared when the Say began).
    v.apply(&SessionEvent::TextDelta("the answer".into()));
    let text = flat(&v);
    assert!(
        !text.contains("1 Step"),
        "the ladder retired the moment the Say began: {text}"
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
    // While the FIRST step streams the ladder holds zero SETTLED steps:
    // no group row, no ladder click targets — the region's OWN header row
    // (`> streaming`, via `live_headers`) is the turn's only click target.
    assert!(
        v.tool_call_headers().is_empty(),
        "no ladder rows exist while the first step streams — nothing to click"
    );
    let live_headers = v.live_headers();
    assert_eq!(
        live_headers.len(),
        1,
        "exactly one live-region header target: {live_headers:?}"
    );
    // Once a step settles the group row becomes clickable again — plain
    // toggle, no pins; the live region below keeps streaming regardless.
    reasoning(&mut v, "step two");
    let headers = v.tool_call_headers();
    assert_eq!(
        headers.len(),
        1,
        "only the settled group row is clickable: {headers:?}"
    );
    v.toggle_tool_call_at(0, 0);
    let headers = v.tool_call_headers();
    assert_eq!(
        headers.len(),
        2,
        "group + step row (calls aggregate waits for step.open): {headers:?}"
    );
    assert_eq!(
        v.live_headers().len(),
        1,
        "the live region keeps its own click target alongside the ladder"
    );
}

/// TWO SURFACES, ONE WINDOW: a user-opened ladder and the live region
/// COEXIST inline — the ladder's settled rows on top, the region's
/// streaming rows directly below; opening the ladder never touches the
/// region (no suppression in either direction), and the user's open state
/// survives the settle: streaming never closes a level behind the user's
/// back. While the FIRST step streams the ladder has no rows to open —
/// the toggle only bites once a step settles.
#[test]
fn user_opened_ladder_coexists_with_the_live_region_and_survives_the_settle() {
    let mut v = ChatView::default();
    reasoning(&mut v, "round one");
    start_call(&mut v, "c1");
    end_call(&mut v, "c1", "out");
    reasoning(&mut v, " round two");
    v.toggle_tool_call_at(0, 0); // open the ladder mid-run (step 1 settled)
    let rows = lines(&v);
    assert_eq!(
        rows[0], "\u{276f} 1 Step  \u{280b} running ",
        "the ladder row counts the SETTLED step and carries the running hint: {rows:?}"
    );
    assert!(
        rows.contains(&"  \u{25b8} Step(1)".to_string()),
        "the ladder renders its settled step row — thinking folded (the step \
itself is closed): {rows:?}"
    );
    assert_eq!(
        rows[3], "> streaming",
        "the region's header rides directly below the ladder's blank: {rows:?}"
    );
    assert!(
        !rows.iter().any(|r| r.contains("round one")),
        "the settled step's thinking stays folded inside the ladder: {rows:?}"
    );
    let lv = live(&v);
    assert!(
        !lv.contains("Step(") && lv.contains("\u{1f4ad} Thinking") && lv.contains("round two"),
        "the LIVE REGION streams the current step forced open — untouched by \
the ladder's disclosure: {lv}"
    );

    reasoning(&mut v, " more");
    assert_eq!(
        disclosure(&v),
        (true, vec![(false, false), (false, false)]),
        "the running step's deltas still open nothing inside the user-open ladder"
    );
    assert!(
        live(&v).ends_with("round two more"),
        "the live region keeps streaming the CURRENT step: {}",
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

/// Ctrl+L folds every collapsible level INCLUDING the live region — but
/// the region never disappears while the turn runs: collapsed it is its
/// own single `▸ streaming` header row (the running step's detail rows fold
/// away), and `toggle_live_region` re-expands it.
#[test]
fn collapse_all_keeps_the_live_region_visible() {
    let mut v = ChatView::default();
    reasoning(&mut v, "thinking");
    v.collapse_all_collapsible();
    assert_eq!(disclosure(&v), (false, vec![(false, false)]));
    assert!(
        v.live_collapsed,
        "Ctrl+L folds the live region with every other level"
    );
    reasoning(&mut v, " more");
    start_call(&mut v, "c1");
    let lv = live(&v);
    assert_eq!(
        lv, "\u{25b8} streaming",
        "collapsed the region is exactly its one header row: {lv}"
    );
    assert_eq!(
        lines(&v),
        vec!["\u{25b8} streaming".to_string()],
        "while the FIRST step streams, the collapsed region row is the \
turn's whole inline surface: {:?}",
        lines(&v)
    );
    v.toggle_live_region();
    assert!(
        !v.live_collapsed && live(&v).contains("thinking more"),
        "toggle_live_region re-expands the detail rows: {}",
        live(&v)
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

/// The region's OWN disclosure: expanded (the default) the `> streaming` header
/// row leads the current step's detail rows; `toggle_live_region` folds it
/// to exactly ONE `▸ streaming` row — the region never vanishes while the turn
/// runs — and toggling back restores the exact same rows (lossless).
#[test]
fn live_region_toggles_between_expanded_and_collapsed_rows() {
    let mut v = ChatView::default();
    reasoning(&mut v, "thinking");
    start_call(&mut v, "c1");
    assert!(!v.live_collapsed, "the region is expanded by default");
    let expanded = lines(&v);
    assert_eq!(
        expanded.first().map(String::as_str),
        Some("> streaming"),
        "the expanded region leads with its header row: {expanded:?}"
    );
    assert!(
        expanded.len() > 1 && live(&v).contains("thinking"),
        "the current step's detail rows follow: {expanded:?}"
    );
    v.toggle_live_region();
    assert!(v.live_collapsed);
    assert_eq!(
        lines(&v),
        vec!["\u{25b8} streaming".to_string()],
        "collapsed the region is EXACTLY its one header row (zero settled \
steps — nothing else renders): {:?}",
        lines(&v)
    );
    v.toggle_live_region();
    assert!(!v.live_collapsed);
    assert_eq!(
        lines(&v),
        expanded,
        "the toggle is lossless — the same rows come back"
    );
}

/// A collapse is the USER's, and it belongs to THAT turn: it persists for
/// the rest of the run (streaming — deltas, calls, even a new step — never
/// re-opens it), but every newly ADMITTED turn (submit / steer / queue
/// echo, i.e. `begin_turn` / `reanchor_turn_after_user_echo`) starts the
/// region expanded again.
#[test]
fn every_newly_admitted_turn_re_expands_the_live_region() {
    let mut v = ChatView::default();
    reasoning(&mut v, "one");
    v.collapse_all_collapsible();
    assert!(
        v.live_collapsed,
        "Ctrl+L collapses the region with the rest"
    );
    reasoning(&mut v, " more");
    start_call(&mut v, "c1");
    reasoning(&mut v, "two");
    assert!(
        v.live_collapsed,
        "mid-run streaming never re-opens the user's collapsed region"
    );
    assert_eq!(
        live(&v),
        "\u{25b8} streaming",
        "and it keeps rendering exactly its collapsed header row: {}",
        live(&v)
    );
    v.apply(&SessionEvent::Done);
    v.begin_turn();
    reasoning(&mut v, "fresh turn");
    assert!(
        !v.live_collapsed,
        "the next admitted turn arms the region expanded again"
    );
    assert!(
        live(&v).contains("fresh turn"),
        "and streams its first step's detail rows: {}",
        live(&v)
    );
    // The echo path (steer/queue consumption) re-arms exactly the same way.
    v.collapse_all_collapsible();
    v.apply(&SessionEvent::Done);
    v.reanchor_turn_after_user_echo();
    reasoning(&mut v, "post-echo turn");
    assert!(
        !v.live_collapsed && live(&v).contains("post-echo turn"),
        "a re-anchored echo turn also starts expanded: {}",
        live(&v)
    );
}

/// Hit-rect alignment: `live_headers()` must report the header line index
/// of the `> streaming` / `▸ streaming` row EXACTLY where `flatten` emits it — with
/// blocks above the running group (markers, settled ladder rows) so the
/// index is non-trivial — and keep pointing at the SAME row when the
/// region collapses (one row instead of many, same position).
#[test]
fn live_header_line_index_points_at_the_rendered_live_row() {
    let mut v = ChatView::default();
    v.push_marker(Line::from("m1"));
    v.push_marker(Line::from("m2"));
    v.begin_turn();
    reasoning(&mut v, "thinking");
    start_call(&mut v, "c1");
    let headers = v.live_headers();
    assert_eq!(headers.len(), 1, "one running group, one live header");
    let rows = lines(&v);
    let idx = headers[0].header_line_idx;
    assert_eq!(
        rows.get(idx).map(String::as_str),
        Some("> streaming"),
        "header_line_idx {idx} must point at the rendered `> streaming` row: {rows:?}"
    );
    assert_eq!(headers[0].block_idx, 2, "the running group sits at index 2");

    // A settled step pushes the region down: group row + blank land between
    // the markers and the region — the header index must track them.
    reasoning(&mut v, "second");
    let headers = v.live_headers();
    let rows = lines(&v);
    assert_eq!(
        rows.get(headers[0].header_line_idx).map(String::as_str),
        Some("> streaming"),
        "after the settle the index still points at the `> streaming` row: {rows:?}"
    );
    assert_eq!(
        headers[0].header_line_idx, 4,
        "markers(2) + group row + blank"
    );

    // Collapsed: same position, different glyph — one row instead of many.
    v.toggle_live_region();
    let headers = v.live_headers();
    let rows = lines(&v);
    assert_eq!(
        rows.get(headers[0].header_line_idx).map(String::as_str),
        Some("\u{25b8} streaming"),
        "collapsed the same index points at the `\u{25b8} streaming` row: {rows:?}"
    );
    assert_eq!(
        rows.len(),
        5,
        "group row + blank + the single region row + markers: {rows:?}"
    );
}
