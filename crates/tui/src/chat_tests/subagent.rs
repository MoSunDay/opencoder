use super::super::*;

fn block_text_for_tick(v: &ChatView, tick: u32) -> String {
    v.flatten_with(tick, 0)
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect()
}

/// With MULTIPLE concurrent subagents, the parent's preamble text stays
/// visible the whole time (Say is never withheld). Each sibling's
/// completion summary surfaces immediately on its own `SubagentEnd`.
#[test]
fn multiple_subagents_keep_preamble_visible() {
    let mut v = ChatView::default();
    // Parent preamble text precedes the subagent dispatch.
    v.apply(&SessionEvent::TextDelta("launching investigators".into()));
    // Two concurrent subagents.
    v.apply(&SessionEvent::SubagentStart {
        id: "a".into(),
        kind: "explore".into(),
        prompt: "p1".into(),
        child_session_id: "ca".into(),
    });
    v.apply(&SessionEvent::SubagentStart {
        id: "b".into(),
        kind: "explore".into(),
        prompt: "p2".into(),
        child_session_id: "cb".into(),
    });

    assert_eq!(v.subagents_running, 2);
    assert!(
        block_text(&v).contains("launching investigators"),
        "preamble stays visible while subagents run"
    );

    // First sibling finishes — its summary surfaces immediately, and the
    // preamble remains on screen.
    v.apply(&SessionEvent::SubagentEnd {
        id: "a".into(),
        ok: true,
        cancelled: false,
        summary: "result-a".into(),
    });
    assert_eq!(v.subagents_running, 1);
    assert!(
        block_text(&v).contains("result-a"),
        "first summary shown immediately while sibling still runs"
    );
    assert!(
        block_text(&v).contains("launching investigators"),
        "preamble still visible while a sibling runs"
    );

    // Last sibling finishes — preamble plus both summaries all visible.
    v.apply(&SessionEvent::SubagentEnd {
        id: "b".into(),
        ok: true,
        cancelled: false,
        summary: "result-b".into(),
    });
    assert_eq!(v.subagents_running, 0);
    let text = block_text(&v);
    assert!(
        text.contains("launching investigators"),
        "preamble visible after all done"
    );
    assert!(text.contains("result-a"), "first summary shown after flush");
    assert!(
        text.contains("result-b"),
        "second summary shown after flush"
    );
}

/// A SINGLE subagent never hides the preamble: its summary surfaces
/// immediately on its own end.
#[test]
fn single_subagent_preamble_visible() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("preamble".into()));
    v.apply(&SessionEvent::SubagentStart {
        id: "s".into(),
        kind: "explore".into(),
        prompt: "p".into(),
        child_session_id: "c".into(),
    });
    assert!(
        block_text(&v).contains("preamble"),
        "preamble still visible"
    );
    // Its summary shows immediately on end (no buffering).
    v.apply(&SessionEvent::SubagentEnd {
        id: "s".into(),
        ok: true,
        cancelled: false,
        summary: "done-single".into(),
    });
    assert!(block_text(&v).contains("done-single"));
}

/// Issue #4: a running subagent header renders the animated spinner glyph
/// (one of the SPINNER frames), not the old static dot `\u{25cf}`.
#[test]
fn running_subagent_renders_spinner_not_dot() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::SubagentStart {
        id: "s".into(),
        kind: "explore".into(),
        prompt: "p".into(),
        child_session_id: "c".into(),
    });
    let text0 = block_text_for_tick(&v, 0);
    let text3 = block_text_for_tick(&v, 3);
    // Neither should contain the old static dot.
    assert!(!text0.contains('\u{25cf}'), "no static dot at tick 0");
    assert!(!text3.contains('\u{25cf}'), "no static dot at tick 3");
    // Tick 0 and tick 3 render different spinner frames (it animates).
    assert_ne!(text0, text3, "spinner frame must change with anim_tick");
}
