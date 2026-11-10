//! The TURN LIVE REGION renders INLINE inside the transcript window:
//! `flatten_with` emits it DIRECTLY BELOW the running group's ladder (the
//! `> streaming` header plus the current step's detail rows) and ABOVE any later
//! blocks (e.g. subagent headers) — one scroll window, one scrollbar,
//! never truncated (scrolling up reveals the region's start); only
//! physical frame overflow clips the tail. The ladder itself renders
//! SETTLED steps only (`steps::settled_steps`): while the FIRST step is
//! still in flight the block emits ZERO rows and the region is the turn's
//! only surface; the region retires when progress settles (Say begins /
//! run ends), leaving the step inside the ladder history.

use super::*;
use crate::chat::ChatView;
use opencoder_session::SessionEvent;
use ratatui::Terminal;

fn row_of(buf: &ratatui::buffer::Buffer, needle: &str, w: u16) -> Option<u16> {
    (0..buf.area.height).find(|&y| row_text(buf, y, w).contains(needle))
}

/// The view's flattened rows as plain strings (span contents joined).
fn flat_lines(v: &ChatView) -> Vec<String> {
    v.flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect()
}

/// The unified scrollbar occupies the inner area's rightmost column; its
/// track/thumb are blank cells with a non-default background, so a painted
/// column proves the scrollbar was drawn.
fn scrollbar_painted(buf: &ratatui::buffer::Buffer, area: ratatui::layout::Rect) -> bool {
    let x = area.right().saturating_sub(2); // inner.right()-1
    (area.y + 1..area.bottom().saturating_sub(1)).any(|y| {
        buf.cell((x, y))
            .is_some_and(|c| c.bg != ratatui::style::Color::Reset)
    })
}

/// Render the body once (`render_body`) at the given size with the given
/// follow flag and return the drawn buffer. The live region is part of the
/// flattened lines now, so no extra wiring is needed.
fn draw_body(v: &ChatView, width: u16, height: u16, follow: bool) -> ratatui::buffer::Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut scroll = 0u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                v,
                &Line::raw("test"),
                &mut scroll,
                follow,
                0,
                0,
                &mut None,
                &mut None,
                &mut None,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut None,
                true,
                0,
                false,
                None,
            );
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

/// A running turn with ONE SETTLED step (round one, call finished) and ONE
/// in-flight step (round two): the ladder shows the settled history while
/// the live region streams the current step below it.
fn running_two_step_view() -> ChatView {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("round one".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "c1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "c1".into(),
        name: "bash".into(),
        output: "done".into(),
        is_error: false,
        images: Vec::new(),
    });
    v.apply(&SessionEvent::ReasoningDelta("round two".into()));
    v
}

/// Flatten order contract: ladder rows < live header < later blocks. The
/// live region streams the running step BELOW the settled ladder and
/// ABOVE any Subagent block that follows, as rows of the SAME flatten
/// output (one scroll window).
#[test]
fn live_region_sits_below_the_ladder_and_above_subagent_blocks() {
    let mut v = running_two_step_view();
    v.apply(&SessionEvent::SubagentStart {
        id: "sa".into(),
        kind: "explore".into(),
        prompt: "dig".into(),
        child_session_id: "cs".into(),
    });

    let flat = flat_lines(&v);
    let ladder = flat
        .iter()
        .position(|l| l.contains("1 Step"))
        .expect("the settled ladder renders its group row");
    let live = flat
        .iter()
        .position(|l| l == "> streaming")
        .expect("the live region renders its `> streaming` header");
    let sub = flat
        .iter()
        .position(|l| l.contains("subagent"))
        .expect("the subagent header renders below");
    assert!(
        ladder < live && live < sub,
        "ladder {ladder} < live {live} < subagent {sub}: {flat:?}"
    );
    // The in-flight step NEVER renders inside the ladder (above the live
    // header) — it streams only in the region below it.
    assert!(
        !flat[..live].iter().any(|l| l.contains("round two")),
        "the in-flight step is not ladder history: {flat:?}"
    );
    assert!(
        flat[live..sub].iter().any(|l| l.contains("round two")),
        "the in-flight step streams in the region: {flat:?}"
    );

    // Same order on screen: the region rides inside the body window,
    // above the `[tok cost]` bottom border.
    let buf = draw_body(&v, 60, 20, false);
    let w = buf.area.width;
    let ladder = row_of(&buf, "1 Step", w).expect("ladder row on screen");
    let live = row_of(&buf, "streaming", w).expect("live header on screen");
    let sub = row_of(&buf, "subagent", w).expect("subagent row on screen");
    let tok = row_of(&buf, "tok", w).expect("[tok cost] border renders");
    assert!(
        ladder < live && live < sub && sub < tok,
        "ladder {ladder} < live {live} < subagent {sub} < tok {tok}"
    );
}

/// While the FIRST step is still in flight there is no ladder history:
/// the group block emits ZERO rows (no `N Steps` row, no trailing blank)
/// and the live region is the running turn's ONLY surface.
#[test]
fn first_in_flight_step_is_the_turns_only_surface() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("pondering".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "c1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    let flat = flat_lines(&v);
    assert_eq!(
        flat.first().map(String::as_str),
        Some("> streaming"),
        "the region's header is the first rendered row: {flat:?}"
    );
    assert!(
        !flat.iter().any(|l| l.contains("Step")),
        "no group row / step counter row while the first step streams: {flat:?}"
    );
    // The region carries the LADDER-IDENTICAL step shape: thinking header +
    // body, the calls aggregate row, the running call's header row.
    let joined = flat.join("\n");
    assert!(joined.contains("\u{1f4ad} Thinking"));
    assert!(joined.contains("1 Function call"));
    assert!(joined.contains("bash") && joined.contains("echo x"));

    // On screen the region fills the window above the `[tok cost]` border.
    let buf = draw_body(&v, 40, 12, false);
    let w = buf.area.width;
    let live = row_of(&buf, "streaming", w).expect("the live header renders");
    let tok = row_of(&buf, "tok", w).expect("[tok cost] border renders");
    assert!(live < tok, "the region renders above the border");
}

/// The OPEN Say is TRANSCRIPT content streaming STANDALONE: the moment it
/// begins the ladder above RETIRES (its `▸ 1 Step` counting row drops out
/// exactly like the live region clears then; the count returns on seal via
/// the merged header), the `say:` header carries the running spinner, and
/// the RAW body rows render below it in full. Sealing merges the pair and
/// dedups the body against the preview.
#[test]
fn streaming_say_renders_standalone_in_the_transcript_until_sealed() {
    let mut v = running_two_step_view();
    v.apply(&SessionEvent::TextDelta("the answer\nand more".into()));

    let flat = flat_lines(&v);
    assert!(
        !flat.iter().any(|l| l.contains("Steps")),
        "the ladder retired the moment the Say began (like the live region): {flat:?}"
    );
    let say = flat
        .iter()
        .position(|l| l.contains("Say:"))
        .expect("the standalone Say: header renders");
    let first = flat
        .iter()
        .position(|l| l.contains("the answer"))
        .expect("the raw first body row renders");
    let rest = flat
        .iter()
        .position(|l| l.contains("and more"))
        .expect("the raw second body row renders");
    assert!(
        say < first && first < rest,
        "say: header, raw body — in order: {flat:?}"
    );
    assert!(
        !flat.iter().any(|l| l == "> streaming"),
        "the live region cleared the moment the Say began: {flat:?}"
    );

    // Seal: the pair MERGES — the retired ladder's count returns as the
    // `Say(n steps)` header and the body dedups its preview line away.
    v.apply(&SessionEvent::Done);
    let flat = flat_lines(&v);
    let merged = flat
        .iter()
        .position(|l| l.contains("Say(2 steps): the answer"))
        .expect("the sealed pair keeps the merged header");
    let body = flat
        .iter()
        .position(|l| l.contains("and more"))
        .expect("the sealed body keeps its rows");
    assert!(
        merged < body,
        "the body still follows the merged header: {flat:?}"
    );
    assert!(
        !flat
            .iter()
            .any(|l| l == "> streaming" || l.contains("running")),
        "sealing drops the region and the spinner — the only visual delta: {flat:?}"
    );
}

/// An EMPTY first Say chunk still OPENS the Say block (the turn closes and
/// the turn floor advances PAST the running ladder) — so the pre-Say
/// surfaces must retire with the BLOCK, not merely on the first non-empty
/// chunk: after the floor advance no later walk can reach the old group to
/// disarm it, and the live region would strand armed beside the streaming
/// Say (and past seal). The Say block landing clears ladder AND live region
/// in the same stroke; the empty body still renders the `say:` header.
#[test]
fn empty_first_say_chunk_still_clears_the_pre_say_surfaces() {
    let mut v = running_two_step_view();
    v.apply(&SessionEvent::TextDelta("".into()));

    let flat = flat_lines(&v);
    assert!(
        !flat.iter().any(|l| l.contains("Steps")),
        "the ladder retired with the Say block: {flat:?}"
    );
    assert!(
        !flat.iter().any(|l| l == "> streaming"),
        "the live region cleared with the Say block (no stranded region): {flat:?}"
    );
    assert!(
        flat.iter().any(|l| l.contains("Say:")),
        "the empty Say still renders its header: {flat:?}"
    );

    // The follow-up non-empty chunk streams the body; sealing the pair keeps
    // the region retired — the merged header renders alone.
    v.apply(&SessionEvent::TextDelta("the answer".into()));
    let flat = flat_lines(&v);
    assert!(
        flat.iter().any(|l| l.contains("the answer")),
        "the body streams below the header: {flat:?}"
    );
    assert!(
        !flat.iter().any(|l| l == "> streaming"),
        "no stranded live region while the Say streams: {flat:?}"
    );
    v.apply(&SessionEvent::Done);
    let flat = flat_lines(&v);
    assert!(
        flat.iter().any(|l| l.contains("Say(2 steps): the answer")),
        "the sealed pair keeps its merged header: {flat:?}"
    );
    assert!(
        !flat.iter().any(|l| l == "> streaming"),
        "the region stays retired after seal: {flat:?}"
    );
}

/// The live content can grow past the frame on its own (untruncated
/// thinking + call rows). The scroll window must keep the TAIL while
/// following (newest rows visible, a scrollbar drawn) AND stay scrollable
/// to the region's START — scroll=0 reveals the `> streaming` header and the
/// first thinking rows: never truncated, never blanked.
#[test]
fn huge_live_rides_the_scroll_window_tail_kept_and_start_reachable() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::QueueConsumed {
        seq: 1,
        text: "go".into(),
    });
    v.apply(&SessionEvent::ReasoningDelta(format!(
        "{}{}",
        "think line\n".repeat(18),
        "a very long thinking line ".repeat(4),
    )));
    v.apply(&SessionEvent::ToolStart {
        id: "c1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    let flat = flat_lines(&v);
    assert!(
        flat.iter().filter(|l| **l == "> streaming").count() == 1,
        "fixture: the region is armed"
    );
    assert!(
        flat.len() > 20,
        "fixture: the region exceeds the 10-row inner area: {flat:?}"
    );

    let buf = draw_body(&v, 60, 12, true);
    let tail_call = row_of(&buf, "bash", 60).expect("the running call's header stays visible");
    let tok = row_of(&buf, "tok", 60).expect("tok cost border");
    assert!(tail_call < tok, "following keeps the region's tail rows");

    let buf = draw_body(&v, 60, 12, false);
    let live = row_of(&buf, "streaming", 60).expect("scroll=0 reveals the live start");
    let think = row_of(&buf, "think line", 60).expect("the first thinking rows render");
    let tok = row_of(&buf, "tok", 60).expect("tok cost border");
    assert!(
        live < think && think < tok,
        "live start, thinking rows in order"
    );
}

/// Regression: the live region must stay VISIBLE when the transcript
/// overflows the frame. Following pins the window to the bottom, so the
/// live rows always land in view — hugging the frame's tail, above the
/// `[tok cost]` border.
#[test]
fn live_strip_stays_visible_when_transcript_overflows() {
    let mut v = ChatView::default();
    for i in 0..12 {
        v.blocks.push(ChatBlock::User {
            rendered: vec![Line::from(format!("user line {i}"))],
        });
    }
    v.begin_turn();
    v.apply(&SessionEvent::ReasoningDelta("pondering".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "c1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });

    let buf = draw_body(&v, 40, 24, true);
    let w = buf.area.width;
    let live = row_of(&buf, "streaming", w).expect("the live header stays visible");
    let call = row_of(&buf, "bash", w).expect("the region's call header renders");
    let tok = row_of(&buf, "tok", w).expect("[tok cost] border renders");
    assert!(live < tok, "region header {live} above tok border {tok}");
    assert!(call < tok, "region call row {call} above tok border {tok}");
    assert!(call > live, "the call rows sit below the region header");
    let user = row_of(&buf, "user line", w).expect("transcript rows fill the frame");
    assert!(user < live, "transcript {user} above the region {live}");
}

/// ONE scrollbar for content + live: when the transcript and the live
/// region TOGETHER overflow the frame, the window draws a scrollbar sized
/// over the unified total (the live rows are inside the cached flatten
/// output), and following shows the live tail rows above the
/// `[tok cost]` border.
#[test]
fn unified_scrollbar_covers_content_and_live() {
    let mut v = ChatView::default();
    for i in 0..12 {
        v.blocks.push(ChatBlock::User {
            rendered: vec![Line::from(format!("user line {i}"))],
        });
    }
    v.begin_turn();
    v.apply(&SessionEvent::ReasoningDelta("pondering".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "c1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });

    let area = ratatui::layout::Rect::new(0, 0, 40, 12);
    let buf = draw_body(&v, 40, 12, true);
    let w = buf.area.width;
    assert!(
        scrollbar_painted(&buf, area),
        "content + live overflow → scrollbar drawn over the unified window"
    );
    let live = row_of(&buf, "streaming", w).expect("the live header stays visible");
    let call = row_of(&buf, "bash", w).expect("the live call header renders");
    let tok = row_of(&buf, "tok", w).expect("[tok cost] border renders");
    assert!(
        live < call && call < tok,
        "live rows ride the window tail: {live} {call} {tok}"
    );
    let user = row_of(&buf, "user line", w).expect("transcript rows fill the rest");
    assert!(user < live, "transcript {user} above the live tail {live}");
}
