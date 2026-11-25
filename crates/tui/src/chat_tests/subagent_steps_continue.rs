//! Regression: "after a subagent the Step is skipped and the Say restarts".
//!
//! A `task` call never enters the ladder, but the `Subagent` block it opens
//! is pushed BETWEEN the turn's ladder and the Say that closes it:
//! `[StepGroup, Subagent, Assistant]`. The pair lookups used to demand strict
//! adjacency, so the ladder kept its `▸ N Steps` row above the subagent row
//! while its own Say streamed standalone below it, and never merged into
//! `Say(n steps): <preview>` on seal. `steps::say_after_group` /
//! `steps::group_before_say` make `Subagent` blocks TRANSPARENT to the pair
//! (render and hit accounting share the lookup); the subagent row renders in
//! block order — merged header, its separator blank, the subagent row, then
//! the Say body.

use super::super::*;
use super::line_accounting::assert_line_accounting_matches;

const SUBAGENT_DONE_ROW: &str =
    "\u{2937} subagent [explore] p \u{2714} done, 0 Steps [\u{2192} view]  child done";

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

fn bash(v: &mut ChatView, id: &str) {
    v.apply(&SessionEvent::ToolStart {
        id: id.into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: id.into(),
        name: "bash".into(),
        output: "out".into(),
        is_error: false,
        images: Vec::new(),
    });
}

/// The symptom sequence: step (bash) → step (task dispatch — the Subagent
/// block lands between the ladder and the Say) → step (bash) → the closing
/// Say, left STREAMING (no `Done` yet).
fn run_to_streaming_say(v: &mut ChatView) {
    v.begin_turn();
    v.apply(&SessionEvent::LlmRoundStart { started_at_ms: 1 });
    v.apply(&SessionEvent::ReasoningDelta("t1".into()));
    bash(v, "b1");
    v.apply(&SessionEvent::LlmRoundEnd);

    v.apply(&SessionEvent::LlmRoundStart { started_at_ms: 2 });
    v.apply(&SessionEvent::ReasoningDelta("t2".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "task1".into(),
        name: "task".into(),
        input: serde_json::json!({"prompt": "p", "kind": "explore"}),
    });
    v.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "p".into(),
        child_session_id: "c1".into(),
    });
    v.apply(&SessionEvent::SubagentEnd {
        id: "s1".into(),
        ok: true,
        cancelled: false,
        summary: "child done".into(),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "task1".into(),
        name: "task".into(),
        output: "child done".into(),
        is_error: false,
        images: Vec::new(),
    });
    v.apply(&SessionEvent::LlmRoundEnd);

    v.apply(&SessionEvent::LlmRoundStart { started_at_ms: 3 });
    v.apply(&SessionEvent::ReasoningDelta("t3".into()));
    bash(v, "b2");
    v.apply(&SessionEvent::LlmRoundEnd);

    v.apply(&SessionEvent::LlmRoundStart { started_at_ms: 4 });
    v.apply(&SessionEvent::TextDelta("final answer".into()));
}

/// One thinking-owned single-call step (the replay fixture's ladder unit).
fn step_with_call(id: &str) -> ChatBlock {
    steps::single_step_group(
        ToolCall {
            id: id.into(),
            header: Line::from(Span::raw(format!("\u{25b8} {id}"))),
            output: Vec::new(),
            started_at_ms: Some(0),
            elapsed_ms: Some(0),
            expanded: false,
        },
        vec![Line::from(format!("think {id}"))],
    )
}

fn subagent_block() -> ChatBlock {
    ChatBlock::Subagent {
        id: "s1".into(),
        child_session_id: "c1".into(),
        kind: "explore".into(),
        prompt: "p".into(),
        view: ChatView::default(),
        done: true,
        ok: true,
        cancelled: false,
        summary: "child done".into(),
        started_at_ms: 0,
        elapsed_ms: Some(0),
    }
}

fn sealed_say(raw: &str) -> ChatBlock {
    ChatBlock::Assistant {
        raw: raw.into(),
        rendered: crate::markdown::render(raw),
        done: true,
    }
}

fn view_with(blocks: Vec<ChatBlock>) -> ChatView {
    ChatView {
        blocks,
        ..ChatView::default()
    }
}

/// Streaming window: the Say behind the subagent row still RETIRES the
/// ladder the moment it begins (no `N Steps` row above the subagent), the
/// subagent row stays in block order, and the Say renders standalone
/// (`say:` header + spinner + raw body). Hit accounting follows: the
/// retired ladder registers no step target and the subagent header is row 0.
#[test]
fn streaming_say_after_a_subagent_retires_the_ladder_behind_the_subagent_row() {
    let mut v = ChatView::default();
    run_to_streaming_say(&mut v);
    assert!(matches!(
        v.blocks.as_slice(),
        [
            ChatBlock::StepGroup { steps, .. },
            ChatBlock::Subagent { done: true, .. },
            ChatBlock::Assistant { done: false, .. }
        ] if steps.len() == 3
    ));
    assert_eq!(
        lines(&v),
        vec![
            SUBAGENT_DONE_ROW,
            "\u{276f} Say: \u{280b} ",
            "    final answer",
        ]
    );
    assert!(
        v.tool_call_headers().is_empty(),
        "the retired ladder registers no step target: {:?}",
        v.tool_call_headers()
    );
    assert_eq!(v.subagent_headers()[0].header_line_idx, 0);
    assert_line_accounting_matches(&v);
}

/// Sealed: the pair merges ACROSS the subagent row — one clickable
/// `▸ Say(3 steps): final answer` header row, its separator blank, the
/// subagent row, and the single-line body hidden (preview dedup). The
/// `Done` boundary still adds its one blank: the subagent row terminates
/// the pair, so `last_block_ends_blank` must not be fooled by the merge.
#[test]
fn sealed_say_after_a_subagent_merges_into_the_ladder_behind_the_subagent_row() {
    let mut v = ChatView::default();
    run_to_streaming_say(&mut v);
    v.apply(&SessionEvent::Done);
    assert!(matches!(
        v.blocks.as_slice(),
        [
            ChatBlock::StepGroup { steps, .. },
            ChatBlock::Subagent { .. },
            ChatBlock::Assistant { done: true, .. },
            ChatBlock::Marker(_)
        ] if steps.len() == 3
    ));
    assert_eq!(
        lines(&v),
        vec![
            "\u{25b8} Say(3 steps): final answer",
            "",
            SUBAGENT_DONE_ROW,
            "",
        ]
    );
    // The merged header row IS the group's click target (row 0); the
    // subagent header sits right below the header + its separator blank.
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].header_line_idx, 0);
    assert_eq!(v.subagent_headers()[0].header_line_idx, 2);
    assert_line_accounting_matches(&v);

    // Opening the ladder re-renders it between the header and the subagent
    // row: header, separator blank, ladder rows + their trailing blank,
    // then the subagent row; the body is still hidden, so nothing follows.
    v.toggle_tool_call_at(0, 0);
    let flat = lines(&v);
    assert_eq!(flat[0], "\u{276f} Say(3 steps): final answer");
    assert_eq!(flat[flat.len() - 2], SUBAGENT_DONE_ROW, "{flat:?}");
    assert_eq!(flat.last().unwrap(), "", "{flat:?}");
    assert_eq!(
        v.subagent_headers()[0].header_line_idx,
        flat.len() - 2,
        "subagent header must sit on its rendered row: {flat:?}"
    );
    assert_line_accounting_matches(&v);
}

/// The `Done` boundary's blank gate is precise about WHERE a hidden-body
/// pair ends: strictly adjacent, the merged header's separator blank closes
/// the pair (true → the boundary must not stack a second blank); behind a
/// subagent row, that row is the last visible line (false → the boundary
/// adds its blank).
#[test]
fn hidden_say_pair_behind_a_subagent_row_does_not_end_blank() {
    let adjacent = view_with(vec![step_with_call("a"), sealed_say("final answer")]);
    assert!(adjacent.last_block_ends_blank());
    let split = view_with(vec![
        step_with_call("a"),
        subagent_block(),
        sealed_say("final answer"),
    ]);
    assert!(!split.last_block_ends_blank());
    // Every OTHER intervening block still breaks the pair.
    let marker_between = view_with(vec![
        step_with_call("a"),
        ChatBlock::Marker(vec![Line::from("")]),
        sealed_say("final answer"),
    ]);
    assert!(!marker_between.last_block_ends_blank());
    assert!(marker_between
        .tool_call_headers()
        .iter()
        .all(|h| h.header_line_idx == 0));
}

/// Replay yields the same shape — child blocks interleave under their
/// parent message and `normalize_turn_groups` caps the ladder at the Say —
/// so the coalesced block order must pair exactly like the live path.
#[test]
fn replayed_ladder_behind_a_subagent_block_pairs_with_its_say_like_live() {
    let mut blocks = vec![
        step_with_call("a"),
        step_with_call("b"),
        subagent_block(),
        step_with_call("c"),
        sealed_say("final answer"),
    ];
    steps_replay::coalesce_steps(&mut blocks);
    assert!(matches!(
        blocks.as_slice(),
        [
            ChatBlock::StepGroup { steps, .. },
            ChatBlock::Subagent { .. },
            ChatBlock::Assistant { done: true, .. }
        ] if steps.len() == 3
    ));
    let v = view_with(blocks);
    assert_eq!(
        lines(&v),
        vec!["\u{25b8} Say(3 steps): final answer", "", SUBAGENT_DONE_ROW,]
    );
    assert_eq!(v.subagent_headers()[0].header_line_idx, 2);
    assert_line_accounting_matches(&v);
}
