use super::super::*;

#[test]
fn plan_handoff_creates_plan_card() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::PlanHandoff(
        "## Plan\n1. do X\n2. do Y".into(),
    ));

    // A Plan block is pushed.
    assert!(
        v.blocks.iter().any(|b| matches!(b, ChatBlock::Plan { .. })),
        "PlanHandoff must create a Plan block"
    );

    // The card renders with a header and the markdown content.
    let flat = v.flatten();
    let text: String = flat
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
        .collect();
    assert!(text.contains("plan"), "plan header must be present");
    assert!(text.contains("Plan"), "plan heading text must be present");
    assert!(text.contains("do X"), "plan content must be present");
    assert!(
        !text.contains("## Plan"),
        "heading markup must be rendered, not raw"
    );
}

#[test]
fn plan_handoff_finalizes_pending_assistant() {
    // An in-progress assistant block must be finalized before the Plan card
    // is pushed, so the plan appears as a separate block.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("partial response".into()));
    v.apply(&SessionEvent::PlanHandoff("## Plan".into()));

    let assistant_count = v
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Assistant { .. }))
        .count();
    assert_eq!(assistant_count, 1, "assistant block must be finalized");
    assert!(
        v.blocks
            .last()
            .map(|b| matches!(b, ChatBlock::Plan { .. }))
            .unwrap_or(false),
        "Plan block must be last"
    );
}

#[test]
fn plan_card_line_count_matches_flatten() {
    // Verify thinking_headers/subagent_headers line counting stays aligned
    // when a Plan block precedes a Thinking block.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::PlanHandoff("line one\nline two".into()));
    v.apply(&SessionEvent::ReasoningDelta("think".into()));

    let flat = v.flatten();
    let headers = v.thinking_headers();
    assert_eq!(headers.len(), 1, "one thinking header expected");
    let line = &flat[headers[0].header_line_idx];
    assert!(
        line.spans.iter().any(|s| s.content.contains("Thinking")),
        "thinking header must point at the correct line"
    );
}

#[test]
fn plan_card_flatten_structure() {
    use ratatui::style::{Color, Modifier};

    crate::theme::set_theme(crate::theme::ThemeKind::Dark);
    let mut v = ChatView::default();
    v.apply(&SessionEvent::PlanHandoff("## Goal\nShip it".into()));

    let flat = v.flatten();

    // Line 0: Yellow bold header "── plan ──".
    let header = &flat[0];
    assert!(
        header.spans.iter().any(|s| s.content.contains("plan")),
        "first line must be the plan header, got: {:?}",
        header.spans
    );
    // Verify the Yellow + Bold styling on the header span.
    assert!(
        header.spans.iter().any(|s| {
            s.style.fg == Some(Color::Yellow) && s.style.add_modifier.contains(Modifier::BOLD)
        }),
        "plan header must be Yellow + Bold"
    );

    // Body lines are indented (start with 2 spaces).
    let body_line = &flat[1];
    assert!(
        body_line
            .spans
            .first()
            .map(|s| s.content.starts_with("  "))
            .unwrap_or(false),
        "body lines must be indented by 2 spaces, got: {:?}",
        body_line.spans
    );

    // Trailing blank line after the body.
    assert!(
        flat.last().map(|l| l.spans.is_empty()).unwrap_or(false),
        "Plan card must end with a trailing blank line"
    );
}

#[test]
fn begin_turn_clears_status() {
    // A transient status set on the previous turn (e.g. an interrupted marker
    // surfaced via SessionEvent::Status) must be cleared at the start of the
    // next turn so it does not leak into the status bar.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::Status("interrupted".into()));
    assert_eq!(v.status, "interrupted");
    v.begin_turn();
    assert!(
        v.status.is_empty(),
        "begin_turn must clear transient status"
    );
    assert!(v.submitted, "begin_turn must set submitted to true");
}

#[test]
fn begin_turn_preserves_transcript() {
    // The turn-start invariant only clears presentation status — the
    // transcript blocks must be untouched.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hello world".into()));
    v.apply(&SessionEvent::Status("interrupted".into()));
    let before = block_text(&v);
    v.begin_turn();
    assert_eq!(
        block_text(&v),
        before,
        "transcript blocks must survive begin_turn"
    );
    assert!(v.status.is_empty());
}

#[test]
fn steer_consumed_echoes_marker_and_drops_entry() {
    // SteerConsumed echoes a ChatBlock::User at consume time (turn boundary)
    // and drops the consumed entry by seq from steer_items. The block is NOT
    // pushed at admit time — it only appears when the steer executes.
    let mut v = ChatView::default();
    v.steer_items.push((7, "use python".into()));
    let before = block_text(&v);
    v.apply(&SessionEvent::SteerConsumed {
        seq: 7,
        text: "use python".into(),
    });
    assert!(
        block_text(&v).contains("User:"),
        "SteerConsumed must echo the User tag at consume time"
    );
    assert!(
        block_text(&v).contains("use python"),
        "SteerConsumed must echo the consumed prompt body"
    );
    assert_ne!(
        block_text(&v),
        before,
        "transcript must change after SteerConsumed echoes"
    );
    assert!(
        v.steer_items.is_empty(),
        "SteerConsumed must drop the consumed entry from steer_items"
    );
}

#[test]
fn steer_consumed_unknown_seq_is_noop() {
    // A SteerConsumed whose seq does not match any pending entry must be a
    // no-op: no marker is pushed and the existing entries are retained.
    let mut v = ChatView::default();
    v.steer_items.push((7, "use python".into()));
    let before = block_text(&v);
    v.apply(&SessionEvent::SteerConsumed {
        seq: 999,
        text: String::new(),
    });
    assert_eq!(block_text(&v), before, "unknown seq must not push a marker");
    assert_eq!(
        v.steer_items.len(),
        1,
        "unknown seq must retain all entries"
    );
}

#[test]
fn last_plan_text_returns_raw_from_plan_block() {
    // When a Plan block exists, last_plan_text must return its `raw` field
    // (the editable markdown source), ignoring any Assistant blocks.
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::Plan {
        rendered: crate::markdown::render("## Plan\n- step one"),
        raw: "## Plan\n- step one".to_string(),
    });
    assert_eq!(
        v.last_plan_text().as_deref(),
        Some("## Plan\n- step one"),
        "last_plan_text must return the Plan block's raw field"
    );
}

#[test]
fn last_plan_text_falls_back_to_assistant_raw() {
    // With no Plan block, last_plan_text falls back to the last non-empty
    // Assistant block's raw — in plan mode the plan IS the last assistant
    // message before the Plan card is handed off.
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::Assistant {
        raw: "first reply".to_string(),
        rendered: crate::markdown::render("first reply"),
        done: true,
    });
    v.blocks.push(ChatBlock::Assistant {
        raw: "second reply".to_string(),
        rendered: crate::markdown::render("second reply"),
        done: true,
    });
    assert_eq!(
        v.last_plan_text().as_deref(),
        Some("second reply"),
        "with no Plan block, last_plan_text must return the last non-empty Assistant raw"
    );
}

#[test]
fn last_plan_text_skips_empty_assistant() {
    // An empty Assistant block must be skipped in favour of the most recent
    // non-empty one.
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::Assistant {
        raw: String::new(),
        rendered: Vec::new(),
        done: false,
    });
    v.blocks.push(ChatBlock::Assistant {
        raw: "real content".to_string(),
        rendered: crate::markdown::render("real content"),
        done: true,
    });
    assert_eq!(
        v.last_plan_text().as_deref(),
        Some("real content"),
        "last_plan_text must skip the empty Assistant and return the non-empty one"
    );
}

#[test]
fn last_plan_text_returns_none_when_empty() {
    // An empty ChatView has nothing to return.
    let v = ChatView::default();
    assert!(
        v.last_plan_text().is_none(),
        "last_plan_text must be None for an empty ChatView"
    );
}

#[test]
fn update_plan_text_updates_plan_block() {
    // When a Plan block exists, update_plan_text rewrites both its `raw` and
    // its `rendered` (markdown re-rendered from the new source).
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::Plan {
        rendered: crate::markdown::render("old plan"),
        raw: "old plan".to_string(),
    });
    v.update_plan_text("new plan text");
    match &v.blocks[0] {
        ChatBlock::Plan { raw, rendered } => {
            assert_eq!(raw, "new plan text", "Plan raw must be updated");
            assert_eq!(
                rendered,
                &crate::markdown::render("new plan text"),
                "Plan rendered must be re-rendered from the new text"
            );
        }
        other => panic!("expected Plan block, got {other:?}"),
    }
}

#[test]
fn update_plan_text_updates_assistant_when_no_plan() {
    // Without a Plan block, update_plan_text edits the last non-empty
    // Assistant block in place: raw is rewritten, rendered is re-rendered,
    // and `done` flips to true.
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::Assistant {
        raw: "original assistant text".to_string(),
        rendered: crate::markdown::render("original assistant text"),
        done: false,
    });
    v.update_plan_text("edited plan via assistant");
    match &v.blocks[0] {
        ChatBlock::Assistant {
            raw,
            rendered,
            done,
        } => {
            assert_eq!(
                raw, "edited plan via assistant",
                "Assistant raw must be updated"
            );
            assert_eq!(
                rendered,
                &crate::markdown::render("edited plan via assistant"),
                "Assistant rendered must be re-rendered"
            );
            assert!(
                *done,
                "done must flip to true after the plan edit is applied"
            );
        }
        other => panic!("expected Assistant block, got {other:?}"),
    }
}
