//! Subagent GROUP disclosure (`ChatView::subagents_collapsed`).
//!
//! Every maximal contiguous run of `ChatBlock::Subagent` blocks opens with
//! ONE group header row — `> subagents` while the run's rows render,
//! `▸ subagents` while collapsed — styled exactly like the TURN LIVE REGION
//! header (`chat_live_turn.rs`: dim warn). Expanded, each row keeps its
//! full-detail shape with NO marker prefix of its own (status, duration,
//! ` [→ view]`, summary); collapsed, the rows are not rendered at all, so
//! `subagent_headers()` is empty and the line count shrinks by one per
//! hidden block while the group header keeps its click target. Unlike the
//! live region the fold is sticky transcript state: it persists across
//! turns and is never re-armed by `begin_turn`.

use super::super::*;
use super::line_accounting::assert_line_accounting_matches;
use ratatui::style::Modifier;

const GROUP_OPEN: &str = "> subagents";
const GROUP_CLOSED: &str = "\u{25b8} subagents";
const ROW: &str = "\u{2937} subagent [explore] ";
const VIEW_HINT: &str = " [\u{2192} view]";

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

fn start(v: &mut ChatView, id: &str, prompt: &str) {
    v.apply(&SessionEvent::SubagentStart {
        id: id.into(),
        kind: "explore".into(),
        prompt: prompt.into(),
        child_session_id: format!("child-{id}"),
    });
}

fn end(v: &mut ChatView, id: &str, summary: &str) {
    v.apply(&SessionEvent::SubagentEnd {
        id: id.into(),
        ok: true,
        cancelled: false,
        summary: summary.into(),
    });
}

/// One finished subagent (with a summary) followed by one still running —
/// a single contiguous run, the two row shapes shown side by side.
fn two_subagents() -> ChatView {
    let mut v = ChatView::default();
    start(&mut v, "s1", "p1");
    end(&mut v, "s1", "child done");
    start(&mut v, "s2", "p2");
    v
}

/// The expanded shape of `two_subagents`: ONE group header, then both rows
/// in full detail (the pre-fold row text, no disclosure prefix).
fn expanded_lines() -> Vec<String> {
    vec![
        GROUP_OPEN.to_string(),
        format!("{ROW}p1 \u{2714} done, 0 Steps{VIEW_HINT}  child done"),
        format!("{ROW}p2 \u{280b} running, 0 Steps 0s{VIEW_HINT}"),
    ]
}

/// Default (expanded): the run opens with the `> subagents` group header,
/// then every row in full detail — status, `[→ view]`, duration, summary —
/// with no marker prefix of its own. Copy mode keeps the header (it is
/// transcript structure, unlike the transient live region).
#[test]
fn subagent_run_renders_the_open_group_header_then_full_rows_by_default() {
    let v = two_subagents();
    assert!(!v.subagents_collapsed, "expanded is the default");
    let flat = lines(&v);
    assert_eq!(flat, expanded_lines(), "{flat:?}");
    for row in &flat[1..] {
        assert!(
            row.starts_with(ROW),
            "rows carry no disclosure marker: {row:?}"
        );
    }
    assert_line_accounting_matches(&v);
    assert!(
        lines_copy(&v).starts_with(&expanded_lines()[..]),
        "copy mode keeps the group header and the full rows"
    );
}

fn lines_copy(v: &ChatView) -> Vec<String> {
    v.flatten_for_copy(0, 0)
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect()
}

/// The group header is styled exactly like the live region header — dim
/// warn — in both disclosure states.
#[test]
fn subagent_group_header_matches_the_live_region_header_style() {
    let expanded = two_subagents();
    let mut collapsed = two_subagents();
    collapsed.toggle_subagents();
    let (open, closed) = (expanded.flatten().remove(0), collapsed.flatten().remove(0));
    assert_eq!(
        open.spans
            .iter()
            .map(|s| s.content.clone())
            .collect::<String>(),
        GROUP_OPEN
    );
    assert_eq!(
        closed
            .spans
            .iter()
            .map(|s| s.content.clone())
            .collect::<String>(),
        GROUP_CLOSED
    );
    for span in [open.spans[0].clone(), closed.spans[0].clone()] {
        assert_eq!(span.style.fg, Some(crate::theme::warn_color()));
        assert!(
            span.style.add_modifier.contains(Modifier::DIM),
            "group header is dim like the live header"
        );
    }
}

/// One toggle collapses the run to the `▸ subagents` header ONLY — no row
/// per block (no status, no view hint, no summary, no duration). Toggling
/// again restores the `> subagents` header plus the full rows.
#[test]
fn toggle_subagents_collapses_the_run_to_its_header_only() {
    let mut v = two_subagents();
    let expanded = lines(&v);
    assert_eq!(
        expanded.len(),
        3,
        "header + one line per block while expanded"
    );

    v.toggle_subagents();
    assert!(v.subagents_collapsed);
    let flat = lines(&v);
    assert_eq!(
        flat,
        vec![GROUP_CLOSED],
        "collapsed run is header-only: {flat:?}"
    );
    assert!(flat[0].ends_with(crate::chat::SUBAGENT_GROUP_LABEL));
    assert_line_accounting_matches(&v);

    v.toggle_subagents();
    assert!(!v.subagents_collapsed);
    assert_eq!(lines(&v), expanded, "re-expanding restores the full rows");
    assert_line_accounting_matches(&v);
}

/// Ctrl+L's `collapse_all_collapsible` sets the collapsed side only. The
/// state is transcript disclosure, not turn state: a new turn's run renders
/// collapsed too (`begin_turn` never re-arms it), and only the group header
/// toggle re-expands it.
#[test]
fn collapse_all_collapsible_collapses_subagents_and_the_state_persists_across_turns() {
    let mut v = ChatView::default();
    start(&mut v, "s1", "p1");
    v.collapse_all_collapsible();
    assert!(v.subagents_collapsed, "Ctrl+L collapses the subagent run");
    assert_eq!(lines(&v), vec![GROUP_CLOSED]);

    // A new turn (behind its user echo) dispatches another subagent: its
    // own run stays collapsed too.
    v.begin_turn();
    v.push_marker(Line::from("next turn"));
    start(&mut v, "s2", "p2");
    assert!(
        v.subagents_collapsed,
        "begin_turn must not re-arm the subagent fold"
    );
    let flat = lines(&v);
    assert_eq!(
        flat,
        vec![GROUP_CLOSED, "next turn", GROUP_CLOSED],
        "{flat:?}"
    );
    assert_line_accounting_matches(&v);

    v.toggle_subagents();
    assert!(!v.subagents_collapsed);
    assert_eq!(
        lines(&v),
        vec![
            GROUP_OPEN.to_string(),
            format!("{ROW}p1 \u{280b} running, 0 Steps 0s{VIEW_HINT}"),
            "next turn".to_string(),
            GROUP_OPEN.to_string(),
            format!("{ROW}p2 \u{280b} running, 0 Steps 0s{VIEW_HINT}"),
        ]
    );
    // Collapsing again (Ctrl+L) is idempotent on an already-collapsed view.
    v.collapse_all_collapsible();
    assert!(v.subagents_collapsed);
    assert_eq!(lines(&v), vec![GROUP_CLOSED, "next turn", GROUP_CLOSED]);
}

/// `subagent_group_headers()` reports ONE target per contiguous run, in both
/// states, pointing at the rendered group header row; `subagent_headers()`
/// locates every row while expanded and is empty while collapsed.
#[test]
fn subagent_group_headers_and_row_headers_track_the_rendered_lines() {
    let mut v = ChatView::default();
    v.push_marker(Line::from("intro"));
    start(&mut v, "s1", "p1");
    end(&mut v, "s1", "child done");
    start(&mut v, "s2", "p2");

    let group = v.subagent_group_headers();
    assert_eq!(group.len(), 1, "one group header per contiguous run");
    assert_eq!(group[0].header_line_idx, 1, "right below the intro marker");
    assert_eq!(lines(&v)[1], GROUP_OPEN);
    let rows = v.subagent_headers();
    assert_eq!(rows.len(), 2, "one row target per block while expanded");
    assert_eq!(rows[0].block_idx, 1);
    assert_eq!(rows[1].block_idx, 2);
    assert_eq!(rows[0].header_line_idx, 2);
    assert_eq!(rows[1].header_line_idx, 3);
    assert!(lines(&v)[2].starts_with(ROW) && lines(&v)[3].starts_with(ROW));
    assert_line_accounting_matches(&v);

    v.toggle_subagents();
    assert_eq!(
        v.subagent_group_headers(),
        group,
        "the group header keeps its target while collapsed"
    );
    assert!(lines(&v)[1] == GROUP_CLOSED);
    assert!(
        v.subagent_headers().is_empty(),
        "no row target while the rows are hidden: {:?}",
        v.subagent_headers()
    );
    assert_eq!(v.flatten().len(), 2);
    assert_line_accounting_matches(&v);

    v.toggle_subagents();
    assert_eq!(
        v.subagent_headers(),
        rows,
        "row targets return with the rows"
    );
    assert_line_accounting_matches(&v);
}

/// Every maximal contiguous run gets its OWN group header — a non-Subagent
/// block between two subagents splits the runs — and a later run's header
/// index shifts with the rows hidden above it while collapsed.
#[test]
fn each_contiguous_run_opens_with_its_own_group_header() {
    let mut v = ChatView::default();
    start(&mut v, "s1", "p1");
    v.push_marker(Line::from("between"));
    start(&mut v, "s2", "p2");
    start(&mut v, "s3", "p3");
    assert_eq!(
        lines(&v),
        vec![
            GROUP_OPEN.to_string(),
            format!("{ROW}p1 \u{280b} running, 0 Steps 0s{VIEW_HINT}"),
            "between".to_string(),
            GROUP_OPEN.to_string(),
            format!("{ROW}p2 \u{280b} running, 0 Steps 0s{VIEW_HINT}"),
            format!("{ROW}p3 \u{280b} running, 0 Steps 0s{VIEW_HINT}"),
        ]
    );
    let groups = v.subagent_group_headers();
    assert_eq!(
        groups.iter().map(|g| g.header_line_idx).collect::<Vec<_>>(),
        vec![0, 3]
    );
    assert_eq!(v.subagent_headers().len(), 3);
    assert_line_accounting_matches(&v);

    v.toggle_subagents();
    assert_eq!(lines(&v), vec![GROUP_CLOSED, "between", GROUP_CLOSED]);
    let groups = v.subagent_group_headers();
    assert_eq!(
        groups.iter().map(|g| g.header_line_idx).collect::<Vec<_>>(),
        vec![0, 2],
        "the second run's header moves up with the hidden rows"
    );
    assert!(v.subagent_headers().is_empty());
    assert_line_accounting_matches(&v);
}
