use super::*;

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

fn block_text_for_tick(v: &ChatView, tick: u32) -> String {
    v.flatten_with(tick, 0)
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect()
}

/// Failure path: when one sibling FAILS (`ok: false`) while another still
/// runs, the failed summary surfaces immediately with its "failed" status
/// and red styling intact. Guards the `ok` flag's round-trip through
/// `mark_subagent_done`.
#[test]
fn failed_subagent_summary_shows_immediately_with_sibling() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("preamble".into()));
    v.apply(&SessionEvent::SubagentStart {
        id: "a".into(),
        kind: "explore".into(),
        prompt: "pa".into(),
        child_session_id: "ca".into(),
    });
    v.apply(&SessionEvent::SubagentStart {
        id: "b".into(),
        kind: "explore".into(),
        prompt: "pb".into(),
        child_session_id: "cb".into(),
    });

    // First sibling FAILS — shown immediately.
    v.apply(&SessionEvent::SubagentEnd {
        id: "a".into(),
        ok: false,
        cancelled: false,
        summary: "crashed".into(),
    });
    assert_eq!(v.subagents_running, 1);
    assert!(
        block_text(&v).contains("crashed"),
        "failed summary shown immediately while sibling runs"
    );

    // Last sibling succeeds — preamble and both summaries all visible.
    v.apply(&SessionEvent::SubagentEnd {
        id: "b".into(),
        ok: true,
        cancelled: false,
        summary: "ok-b".into(),
    });
    let text = block_text(&v);
    assert!(text.contains("crashed"), "failed summary still shown");
    assert!(text.contains("ok-b"), "ok summary shown");
    assert!(text.contains("preamble"), "preamble visible");
    // Status words reflect each subagent's outcome.
    assert!(text.contains("failed"), "failed subagent shows 'failed'");
    assert!(text.contains("done"), "ok subagent shows 'done'");
}

/// Safety flush: if a turn ends (`Done`) while subagents are still marked
/// running (e.g. interrupted mid-dispatch), the running count must reset so
/// the status-bar badge recovers. The preamble stays visible throughout.
#[test]
fn done_while_subagents_running_resets_count() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("preamble".into()));
    v.apply(&SessionEvent::SubagentStart {
        id: "a".into(),
        kind: "explore".into(),
        prompt: "pa".into(),
        child_session_id: "ca".into(),
    });
    v.apply(&SessionEvent::SubagentStart {
        id: "b".into(),
        kind: "explore".into(),
        prompt: "pb".into(),
        child_session_id: "cb".into(),
    });
    assert_eq!(v.subagents_running, 2);

    // Turn ends abnormally — no SubagentEnd events arrived.
    v.apply(&SessionEvent::Done);

    assert_eq!(v.subagents_running, 0, "Done must reset running count");
    assert!(
        block_text(&v).contains("preamble"),
        "preamble visible after Done"
    );
}

/// Subagent events render correctly: SubagentStart creates a foldable
/// block, child events route into its inner view (not the parent), and
/// SubagentEnd marks done + shows the summary. Parent context excludes
/// child tokens.
#[test]
fn subagent_events_render() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("parent asks subagent".into()));
    v.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "search".into(),
        child_session_id: "sub-1".into(),
    });
    assert!(block_text(&v).contains("subagent"));
    assert!(block_text(&v).contains("explore"));
    assert_eq!(v.subagents_total, 1);
    assert_eq!(v.subagents_running, 1);

    // Child events routed into the subagent block's view.
    let parent_ctx = v.context_used;
    assert!(parent_ctx > 0, "precondition: parent has its own tokens");
    v.apply(&SessionEvent::SubagentChild {
        id: "s1".into(),
        ev: Box::new(SessionEvent::TextDelta("child output".into())),
    });
    // Finalize the child's assistant block so its tokens are counted
    // (counted at finalization, not per-delta, to keep the status bar's
    // ctx% indicator stable).
    v.apply(&SessionEvent::SubagentChild {
        id: "s1".into(),
        ev: Box::new(SessionEvent::Done),
    });
    assert_eq!(
        v.context_used, parent_ctx,
        "parent must not include child tokens"
    );
    assert!(!block_text(&v).contains("child output"));
    if let Some(ChatBlock::Subagent { view, .. }) = v
        .blocks
        .iter()
        .find(|b| matches!(b, ChatBlock::Subagent { .. }))
    {
        assert!(block_text(view).contains("child output"));
        assert!(view.context_used > 0);
    } else {
        panic!("expected a Subagent block");
    }

    v.apply(&SessionEvent::SubagentEnd {
        id: "s1".into(),
        ok: true,
        cancelled: false,
        summary: "found it".into(),
    });
    assert_eq!(v.subagents_running, 0);
    assert_eq!(v.subagents_total, 1);
    assert!(block_text(&v).contains("found it"));
    assert_eq!(v.context_used, parent_ctx + estimate("found it") as u64);
}

/// A steer admitted to a running child (via `SubagentSteer`) that the child
/// never absorbed (it finished first) must be cleared when the child ends —
/// otherwise the leftover row would sit on the pending panel forever.
#[test]
fn subagent_end_clears_leftover_child_steer_rows() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "p1".into(),
        child_session_id: "cs1".into(),
    });
    // Child steer admitted while running, never absorbed.
    if let Some(ChatBlock::Subagent { view, .. }) = v.blocks.last_mut() {
        view.steer_items.push((1, "late steer".into()));
    }
    assert_eq!(
        v.blocks
            .iter()
            .filter_map(|b| match b {
                ChatBlock::Subagent { view, .. } => Some(view.steer_items.len()),
                _ => None,
            })
            .sum::<usize>(),
        1,
        "leftover steer must be present before the child ends"
    );

    v.apply(&SessionEvent::SubagentEnd {
        id: "s1".into(),
        ok: true,
        cancelled: false,
        summary: "done".into(),
    });

    assert!(
        v.blocks
            .iter()
            .filter_map(|b| match b {
                ChatBlock::Subagent { view, .. } => Some(view.steer_items.len()),
                _ => None,
            })
            .all(|n| n == 0),
        "completed subagent must not retain leftover steer rows"
    );
}

/// A subagent whose `SubagentEnd` was dropped under channel saturation must
/// be reconciled (marked interrupted) when `Done` arrives, so no phantom
/// spinning block outlives the turn - otherwise the mode-switch running-gate
/// would let a Shift+Tab switch slip through while a dead block still looks
/// "running".
#[test]
fn done_reconciles_orphaned_subagent_blocks() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::SubagentStart {
        id: "a".into(),
        kind: "explore".into(),
        prompt: "p".into(),
        child_session_id: "ca".into(),
    });
    // NOTE: deliberately NO SubagentEnd - simulates a dropped event.
    assert_eq!(v.subagents_running, 1);
    v.apply(&SessionEvent::Done);
    assert_eq!(v.subagents_running, 0, "Done zeroes the running counter");
    let block = v
        .blocks
        .iter()
        .rev()
        .find(|b| matches!(b, ChatBlock::Subagent { id, .. } if id == "a"));
    let sb = block.expect("subagent block must still exist");
    match sb {
        ChatBlock::Subagent {
            done,
            ok,
            cancelled,
            summary,
            ..
        } => {
            assert!(*done, "orphaned subagent must be marked done on Done");
            assert!(!*ok, "orphaned subagent must be marked failed (not ok)");
            assert!(
                !*cancelled,
                "orphaned subagent is interrupted, not user-cancelled"
            );
            assert!(
                summary.contains("interrupted"),
                "summary should mention interrupted; got {:?}",
                summary
            );
        }
        _ => unreachable!(),
    }
}

/// Error also reconciles orphaned subagent blocks (same lossy-channel risk).
#[test]
fn error_reconciles_orphaned_subagent_blocks() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::SubagentStart {
        id: "b".into(),
        kind: "build".into(),
        prompt: "p".into(),
        child_session_id: "cb".into(),
    });
    v.apply(&SessionEvent::Error("boom".into()));
    assert_eq!(v.subagents_running, 0);
    let sb = v
        .blocks
        .iter()
        .rev()
        .find(|b| matches!(b, ChatBlock::Subagent { id, .. } if id == "b"))
        .unwrap();
    match sb {
        ChatBlock::Subagent { done, summary, .. } => {
            assert!(*done);
            assert!(summary.contains("interrupted"));
        }
        _ => unreachable!(),
    }
}
