//! Subagent GROUP accounting mirror: one `> subagents` / `▸ subagents`
//! header line per maximal contiguous run of `Subagent` blocks (both
//! disclosure states) plus one row line per block only while EXPANDED —
//! `collect_headers` and `flatten_with` must agree in both states so the
//! hit-rects below a run stay aligned.

use super::*;

/// Build a finished subagent block (empty child view).
fn subagent_done(id: &str) -> ChatBlock {
    ChatBlock::Subagent {
        id: id.into(),
        child_session_id: format!("child-{id}"),
        kind: "explore".into(),
        prompt: "p".into(),
        view: ChatView::default(),
        done: true,
        ok: true,
        cancelled: false,
        summary: String::new(),
        started_at_ms: 0,
        elapsed_ms: Some(0),
    }
}

fn line_texts(v: &ChatView) -> Vec<String> {
    v.flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect()
}

#[test]
fn subagent_group_header_and_rows_align_in_both_states() {
    // Two runs split by a marker; a step group below proves later hit-rects.
    let v = view_with(vec![
        subagent_done("a"),
        subagent_done("b"),
        marker_n(1),
        subagent_done("c"),
        step_group_closed(),
    ]);
    assert_line_accounting_matches(&v);
    // header + 2 rows + marker + header + row + group row + blank = 8
    assert_eq!(v.flatten().len(), 8);
    assert_eq!(
        v.subagent_group_headers()
            .iter()
            .map(|g| g.header_line_idx)
            .collect::<Vec<_>>(),
        vec![0, 4]
    );
    assert_eq!(
        v.subagent_headers()
            .iter()
            .map(|h| h.header_line_idx)
            .collect::<Vec<_>>(),
        vec![1, 2, 5]
    );
    let flat = line_texts(&v);
    assert_eq!(flat[0], "> subagents");
    assert_eq!(flat[4], "> subagents");
    assert!(flat[1].starts_with("\u{2937} subagent"));

    let mut collapsed = view_with(v.blocks.clone());
    collapsed.subagents_collapsed = true;
    assert_line_accounting_matches(&collapsed);
    // header + marker + header + group row + blank = 5
    assert_eq!(collapsed.flatten().len(), 5);
    assert!(
        collapsed.subagent_headers().is_empty(),
        "no row while collapsed"
    );
    let flat = line_texts(&collapsed);
    assert_eq!(flat[0], "\u{25b8} subagents");
    assert_eq!(flat[2], "\u{25b8} subagents");
    // Hit-rects below the hidden rows stay aligned on the shifted lines.
    let tool_idx = collapsed.tool_call_headers()[0].header_line_idx;
    assert_eq!(tool_idx, 3, "the group row follows the second header");
    assert!(flat[tool_idx].contains("1 Step"));
}
