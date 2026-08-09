use super::super::*;

fn make_subagent(started_at_ms: i64, elapsed_ms: Option<u64>, done: bool) -> ChatBlock {
    ChatBlock::Subagent {
        id: "s1".into(),
        child_session_id: "c1".into(),
        kind: "explore".into(),
        prompt: "find foo".into(),
        view: ChatView::default(),
        done,
        ok: done,
        cancelled: false,
        summary: if done {
            "found it".into()
        } else {
            String::new()
        },
        started_at_ms,
        elapsed_ms,
    }
}

fn flat_text(v: &ChatView, now_ms: i64) -> String {
    v.flatten_with(0, now_ms)
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect()
}

// --- Subagent timer tests ---

#[test]
fn running_subagent_shows_live_timer() {
    let mut v = ChatView::default();
    v.blocks.push(make_subagent(1000, None, false));
    let text = flat_text(&v, 6000);
    assert!(
        text.contains("5s"),
        "running subagent should show live timer; got: {text}"
    );
}

#[test]
fn done_subagent_freezes_duration() {
    let mut v = ChatView::default();
    v.blocks.push(make_subagent(1000, Some(18000), true));
    let text = flat_text(&v, 100000);
    assert!(
        text.contains("18s"),
        "done subagent should show frozen 18s; got: {text}"
    );
}

#[test]
fn done_subagent_hides_subsecond() {
    let mut v = ChatView::default();
    v.blocks.push(make_subagent(1000, Some(500), true));
    let text = flat_text(&v, 100000);
    assert!(
        !text.contains("0s"),
        "sub-second done subagent duration should be hidden; got: {text}"
    );
}

// --- Self-heal: recover missing round anchor when LlmRoundStart was dropped ---

/// When the round-start anchor is `None` (LlmRoundStart dropped by a saturated
/// channel), a `TextDelta` must re-anchor the timer so `[turn cost]` still shows.
#[test]
fn streaming_delta_self_heals_missing_round_start() {
    let mut v = ChatView::default();
    assert_eq!(v.llm_round_started_at_ms, None);
    v.apply(&SessionEvent::TextDelta("hi".into()));
    assert!(
        v.llm_round_started_at_ms.is_some(),
        "TextDelta must self-heal a missing round anchor; still None"
    );
    assert!(
        block_text(&v).contains("hi"),
        "delta text must still be appended alongside self-heal"
    );
}

/// A genuine `LlmRoundStart` (runner timestamp) must NOT be overwritten by the
/// later `TextDelta` self-heal — only fill when the anchor is `None`.
#[test]
fn explicit_round_start_not_overwritten() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 1000,
    });
    v.apply(&SessionEvent::TextDelta("hi".into()));
    assert_eq!(
        v.llm_round_started_at_ms,
        Some(1000),
        "self-heal must not clobber the runner-provided round start"
    );
}

/// `ReasoningDelta` follows the same self-heal path as `TextDelta`.
#[test]
fn reasoning_delta_also_self_heals() {
    let mut v = ChatView::default();
    assert_eq!(v.llm_round_started_at_ms, None);
    v.apply(&SessionEvent::ReasoningDelta("thinking".into()));
    assert!(
        v.llm_round_started_at_ms.is_some(),
        "ReasoningDelta must self-heal a missing round anchor; still None"
    );
}

/// A self-healed anchor is cleared normally by `LlmRoundEnd`.
#[test]
fn round_end_clears_self_healed_anchor() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hi".into()));
    assert!(
        v.llm_round_started_at_ms.is_some(),
        "self-heal established anchor"
    );
    v.apply(&SessionEvent::LlmRoundEnd);
    assert_eq!(
        v.llm_round_started_at_ms, None,
        "LlmRoundEnd must clear a self-healed anchor"
    );
}

/// A delta routed into a subagent via `SubagentChild` self-heals the CHILD
/// view's anchor (the recursive `view.apply(ev)` path), so `[turn cost]` is
/// recovered inside the subagent fold too.
#[test]
fn subagent_child_delta_self_heals_child_view() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "find foo".into(),
        child_session_id: "c1".into(),
    });
    // Child has produced no round-start event (simulating a dropped one).
    v.apply(&SessionEvent::SubagentChild {
        id: "s1".into(),
        ev: Box::new(SessionEvent::TextDelta("child reply".into())),
    });
    let child_anchor = v
        .blocks
        .iter()
        .rev()
        .find_map(|b| match b {
            ChatBlock::Subagent { view, .. } => Some(view.llm_round_started_at_ms),
            _ => None,
        })
        .flatten();
    assert!(
        child_anchor.is_some(),
        "SubagentChild(TextDelta) must self-heal the child view's round anchor; still None"
    );
}

/// The turn timer anchor must survive LlmRoundEnd so [turn cost] does not
/// disappear in the gap between rounds.
#[test]
fn turn_anchor_survives_round_end() {
    let mut v = ChatView::default();
    v.begin_turn();
    assert!(
        v.turn_started_at_ms.is_some(),
        "begin_turn must set the turn anchor"
    );
    v.apply(&SessionEvent::LlmRoundStart { started_at_ms: 1000 });
    v.apply(&SessionEvent::LlmRoundEnd);
    assert!(
        v.turn_started_at_ms.is_some(),
        "turn anchor must survive LlmRoundEnd"
    );
    v.apply(&SessionEvent::Done);
    assert!(
        v.turn_started_at_ms.is_none(),
        "Done must clear the turn anchor"
    );
}
