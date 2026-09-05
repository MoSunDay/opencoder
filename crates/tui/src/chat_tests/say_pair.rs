//! The ADJACENT Say-pair contract: when a `StepGroup`'s next block is its
//! turn's Say, the standalone `N Steps` row and the `❯ Say:` header fold
//! into ONE clickable merged header `{glyph} Say(n step{s}): <preview>` —
//! the count, the preview and (while the Say streams last) the running hint
//! all live on that row. The header is followed by exactly ONE blank before
//! the body, and the body SKIPS its first non-empty line when it duplicates
//! the preview (a single-line Say renders body-hidden: header + blank only).
//! Pins the layout, the blank discipline, the body dedup, the click/toggle
//! wiring, the non-adjacent fallback (old two-row layout), the pure-text
//! Say, the stale boundary marker, the per-sub-turn step counting, and the
//! line-accounting mirror.

use super::super::*;

fn call_tool(v: &mut ChatView, id: &str) {
    v.apply(&SessionEvent::ToolStart {
        id: id.into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: id.into(),
        name: "bash".into(),
        output: format!("{id}-out"),
        is_error: false,
        images: Vec::new(),
    });
}

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

/// A done tool turn's collapsed pair: ONE merged header row, then ONE blank
/// (the header never squeezes against the body), then the Say body with its
/// first line skipped (the preview on the header already shows it), then
/// exactly one trailing blank. The preview is the Say's first non-empty line;
/// the label keeps the role-header styling.
#[test]
fn closed_pair_renders_one_merged_header_row() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("thinking".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta(
        "the final answer\nsecond line".into(),
    ));
    v.apply(&SessionEvent::Done);

    let flat = lines(&v);
    assert_eq!(
        flat,
        vec![
            "\u{25b8} Say(1 step): the final answer",
            "",
            "    second line",
            "",
        ],
        "merged header + blank + deduped body + one trailing blank: {flat:?}"
    );
    let header = &v.flatten()[0];
    assert_eq!(header.spans[0].content, "\u{25b8} Say(1 step): ");
    assert!(
        header.spans[0]
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD),
        "the Say label is styled like the role header"
    );
    assert_eq!(header.spans[1].content, "the final answer");
}

/// Clicking the merged header toggles the ladder (the pair exposes ONE
/// target on the merged row); clicking again closes it back.
#[test]
fn merged_header_click_toggles_the_ladder() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("thinking".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta("answer".into()));
    v.apply(&SessionEvent::Done);

    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 1, "collapsed pair exposes ONE target");
    assert_eq!(headers[0].call_idx, 0);
    assert_eq!(
        headers[0].header_line_idx, 0,
        "the hit rect rides the merged header row"
    );

    v.toggle_tool_call_at(headers[0].block_idx, headers[0].call_idx);
    let open = lines(&v);
    assert!(
        open[0].starts_with("\u{276f} Say(1 step)"),
        "open pair flips the glyph on the SAME merged row: {open:?}"
    );
    assert_eq!(open[1], "", "header row is followed by ONE blank: {open:?}");
    assert!(
        open.iter().any(|l| l.contains("Step(1)")),
        "open pair shows the step rows under the merged header: {open:?}"
    );

    v.toggle_tool_call_at(headers[0].block_idx, headers[0].call_idx);
    assert_eq!(
        lines(&v),
        vec!["\u{25b8} Say(1 step): answer", ""],
        "second click collapses back: single-line Say renders header + ONE blank (body hidden)"
    );
}

/// Line-accounting mirror stays exact for merged pairs in every disclosure
/// state (this is what keeps mouse hit-rects aligned with the render).
#[test]
fn merged_pair_line_accounting_stays_exact() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    call_tool(&mut v, "t1");
    call_tool(&mut v, "t2");
    v.apply(&SessionEvent::TextDelta("answer".into()));
    v.apply(&SessionEvent::Done);
    super::line_accounting::assert_line_accounting_matches(&v);

    // Open ladder + expanded calls: the heaviest merged layout still accounts.
    v.toggle_tool_call_at(0, 0);
    v.toggle_tool_call_at(0, 1);
    v.toggle_tool_call_at(0, 2);
    v.toggle_tool_call_at(0, 3);
    super::line_accounting::assert_line_accounting_matches(&v);

    v.collapse_all_collapsible();
    super::line_accounting::assert_line_accounting_matches(&v);
}

/// Ctrl+L closes the merged pair's ladder; the merged header glyph flips
/// back to the closed prefix.
#[test]
fn ctrl_l_collapses_the_merged_pair() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta("answer".into()));
    v.apply(&SessionEvent::Done);
    v.toggle_tool_call_at(0, 0);
    assert!(lines(&v).iter().any(|l| l.contains("Step(1)")));

    v.collapse_all_collapsible();
    assert_eq!(
        lines(&v),
        vec!["\u{25b8} Say(1 step): answer", ""],
        "Ctrl+L folds the pair back to its single merged header row (+ blank)"
    );
}

/// A Say with NO preceding group (pure-text turn) keeps the standalone
/// `❯ Say:` header — the merge only applies to group+Say pairs.
#[test]
fn pure_text_say_keeps_the_standalone_header() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("plain speech".into()));
    v.apply(&SessionEvent::Done);
    assert_eq!(
        lines(&v),
        vec!["\u{276f} Say:", "    plain speech", ""],
        "no group -> no merge: the classic Say header row survives"
    );
}

/// A Marker between the group and the Say un-pairs them: the old
/// `N Steps` + `❯ Say:` two-row layout returns for both blocks.
#[test]
fn non_adjacent_say_keeps_the_old_layout() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::Done);
    // A system marker lands between the closed ladder and the next run's
    // Say: the group's next block is a Marker, not the Say.
    v.push_marker(ratatui::text::Line::from("system note"));
    v.begin_turn();
    v.apply(&SessionEvent::TextDelta("spoken".into()));
    v.apply(&SessionEvent::Done);

    let flat = lines(&v);
    assert!(
        flat.iter().any(|l| l.contains("\u{25b8} 1 Step")),
        "standalone group row survives a non-adjacent Say: {flat:?}"
    );
    assert!(
        flat.iter().any(|l| *l == "\u{276f} Say:"),
        "standalone Say header survives a preceding marker: {flat:?}"
    );
    let group = v
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::StepGroup { .. }))
        .unwrap();
    // The group's own target rides its standalone row; the Say that follows
    // the marker renders its own header line above its body.
    let say_line = flat.iter().position(|l| *l == "\u{276f} Say:").unwrap();
    let group_line = v
        .tool_call_headers()
        .iter()
        .find(|h| h.block_idx == group)
        .map(|h| h.header_line_idx)
        .unwrap();
    assert!(
        group_line < say_line,
        "group row precedes the Say: {flat:?}"
    );
}

/// A post-Say fresh ladder must not slip between the previous Say's body
/// and its boundary blank: the insert floor skips a blank Marker sitting at
/// the turn boundary, so the stale marker keeps separating the pairs.
#[test]
fn fresh_ladder_lands_below_a_boundary_blank_marker() {
    let mut v = ChatView::default();
    v.begin_turn();
    v.apply(&SessionEvent::ReasoningDelta("round one".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta("first say".into()));
    v.apply(&SessionEvent::Done);
    let after_first = lines(&v);

    v.begin_turn();
    v.apply(&SessionEvent::ReasoningDelta("round two".into()));
    call_tool(&mut v, "t2");
    v.apply(&SessionEvent::TextDelta("second say".into()));
    v.apply(&SessionEvent::Done);

    let flat = lines(&v);
    // The first pair keeps its exact shape (header + body + one blank); the
    // second pair starts AFTER that blank, not inside it.
    let prefix: Vec<String> = flat[..after_first.len()].to_vec();
    assert_eq!(prefix, after_first, "pair one is untouched by turn two");
    let blank_after_first = &flat[after_first.len() - 1];
    assert!(
        blank_after_first.trim().is_empty(),
        "exactly one blank separates the pairs: {flat:?}"
    );
    assert!(
        flat.iter()
            .skip(after_first.len())
            .any(|l| l.contains("Say(1 step): second say")),
        "the second pair renders its own merged header: {flat:?}"
    );
}

/// Copy mode keeps the merged pair header's PREVIEW payload: the Say's
/// first line lives ONLY on that header (the rendered body skips it, and a
/// single-line Say renders body-hidden), so the header is not role chrome —
/// only its label span is (see copy_mode::clean::say_pair_payload).
#[test]
fn copy_mode_keeps_the_merged_header_preview() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta("answer".into()));
    v.apply(&SessionEvent::Done);

    let flat = v.flatten();
    let header = &flat[0];
    assert_eq!(
        crate::copy_mode::clean::clean_line(header).as_deref(),
        Some("answer"),
        "the merged header carries the Say's only rendering of its first line"
    );
}
