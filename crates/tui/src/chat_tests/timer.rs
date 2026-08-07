use super::super::*;
use crate::theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

fn make_tool(started_at_ms: i64, elapsed_ms: Option<u64>) -> ChatBlock {
    ChatBlock::Tool {
        id: "t1".into(),
        header: Line::from(vec![
            Span::styled(
                "\u{25b8} bash ",
                Style::default()
                    .fg(theme::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("ls -la", Style::default().fg(theme::muted())),
        ]),
        output: Vec::new(),
        collapsed: true,
        started_at_ms,
        elapsed_ms,
    }
}

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
        summary: if done { "found it".into() } else { String::new() },
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

fn find_duration_span(v: &ChatView, now_ms: i64, needle: &str) -> Option<ratatui::style::Style> {
    v.flatten_with(0, now_ms)
        .iter()
        .flat_map(|l| l.spans.iter())
        .find(|s| s.content.contains(needle))
        .map(|s| s.style)
}

// --- Tool timer tests ---

#[test]
fn running_tool_shows_live_timer() {
    let mut v = ChatView::default();
    v.blocks.push(make_tool(1000, None));
    let text = flat_text(&v, 4000);
    assert!(
        text.contains("3s"),
        "running tool should show live timer; got: {text}"
    );
}

#[test]
fn running_tool_timer_updates_with_now() {
    let mut v = ChatView::default();
    v.blocks.push(make_tool(1000, None));
    assert!(flat_text(&v, 2000).contains("1s"));
    assert!(flat_text(&v, 11000).contains("10s"));
}

#[test]
fn running_tool_uses_warn_color() {
    let mut v = ChatView::default();
    v.blocks.push(make_tool(1000, None));
    let style = find_duration_span(&v, 4000, "3s")
        .expect("should find duration span");
    assert_eq!(
        style.fg,
        Some(theme::warn_color()),
        "running timer should use warn color"
    );
}

#[test]
fn done_tool_freezes_duration() {
    let mut v = ChatView::default();
    v.blocks.push(make_tool(1000, Some(5000)));
    let text = flat_text(&v, 100000);
    assert!(
        text.contains("5s"),
        "done tool should show frozen 5s; got: {text}"
    );
}

#[test]
fn done_tool_uses_muted_color() {
    let mut v = ChatView::default();
    v.blocks.push(make_tool(1000, Some(5000)));
    let style = find_duration_span(&v, 100000, "5s")
        .expect("should find duration span");
    assert_eq!(
        style.fg,
        Some(theme::muted()),
        "done timer should use muted color"
    );
}

#[test]
fn done_tool_hides_subsecond() {
    let mut v = ChatView::default();
    v.blocks.push(make_tool(1000, Some(500)));
    let text = flat_text(&v, 100000);
    assert!(
        !text.contains("0s"),
        "sub-second done duration should be hidden; got: {text}"
    );
}

#[test]
fn running_tool_shows_zero_when_just_started() {
    let mut v = ChatView::default();
    v.blocks.push(make_tool(1000, None));
    let text = flat_text(&v, 1000);
    assert!(
        text.contains("0s"),
        "running tool should always show timer; got: {text}"
    );
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
