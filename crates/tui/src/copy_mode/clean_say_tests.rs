//! End-to-end copy-mode regressions for the merged Say pair header: Ctrl+G
//! renders the transcript through `clean::clean_line`, and the pair header
//! `{❯|▸} Say(n step{s}): <preview>` is the ONLY rendering of the Say's
//! first line (the body below skips it — preview dedup — and a single-line
//! Say renders body-hidden entirely), so its preview payload must survive
//! with the label/spinner chrome stripped. Also pins the user requirement
//! that an EXPANDED ladder's step-thinking body and call output stay
//! visible in copy mode.

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

use crate::chat::ChatView;
use opencoder_session::SessionEvent;

/// One bash tool-call round-trip (`t{id}` → output `{id}-out`), the same
/// event shape `chat_tests::say_pair` drives.
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

/// Draw `view` through [`super::render_clean`] on a `w`×`h` terminal in
/// follow mode and return the terminal for buffer inspection (mirror of
/// the in-module `draw_clean` fixture, with the size as a parameter).
fn draw_clean_sized(view: &ChatView, w: u16, h: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut scroll = 0u32;
    let mut viewport = None;
    terminal
        .draw(|f| {
            super::render_clean(
                f,
                f.area(),
                view,
                &mut scroll,
                true,
                0,
                0,
                &mut viewport,
                None,
            );
        })
        .unwrap();
    terminal
}

/// All cell symbols of a drawn buffer joined into one string.
fn buf_text(buf: &Buffer) -> String {
    buf.content
        .iter()
        .flat_map(|c| c.symbol().chars())
        .collect()
}

/// The copy-mode buffer text of `view` on a 40×20 terminal (tall enough
/// that follow mode never scrolls the tail away).
fn copy_text(view: &ChatView) -> String {
    buf_text(draw_clean_sized(view, 40, 20).backend().buffer())
}

/// Flattened decorated rows as plain text (for pre-render preconditions).
fn row_texts(v: &ChatView) -> Vec<String> {
    v.flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect()
}

#[test]
fn single_line_say_survives_copy_mode() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("thinking".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta("the answer".into()));
    v.apply(&SessionEvent::Done);

    let text = copy_text(&v);
    assert!(
        text.contains("the answer"),
        "a single-line Say lives ONLY on the pair header — its preview must survive: {text}"
    );
    assert!(
        !text.contains("Say(1 step)"),
        "the label chrome is stripped from the copied payload: {text}"
    );
}

#[test]
fn multi_line_say_shows_first_line_and_rest() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("thinking".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta(
        "the final answer\nsecond line".into(),
    ));
    v.apply(&SessionEvent::Done);

    let text = copy_text(&v);
    assert!(
        text.contains("the final answer"),
        "the body's skipped first line must survive via the header preview: {text}"
    );
    assert!(
        text.contains("second line"),
        "the body rows below the skipped first line must survive: {text}"
    );
}

#[test]
fn streaming_say_streams_in_the_live_region_not_copy_text() {
    // No final Done: the Say is still OPEN, so it is not transcript content
    // at all — `flatten` skips it and it streams only in the TURN LIVE
    // REGION. Copy mode copies the transcript, so neither the payload nor
    // any spinner chrome may appear while it streams; the payload lands in
    // copy text only once sealed (see `multi_line_say_shows_first_line_and_rest`).
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("thinking".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta("the answer".into()));

    let live = crate::chat::live_turn::live_data(&v.blocks, v.turn_block_start)
        .map(|live| crate::chat::live_turn::render_lines(live.steps, live.say, 0))
        .unwrap_or_default();
    let live_text = live
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        live_text.contains("\u{1f4ac} Say") && live_text.contains("the answer"),
        "the open Say streams in the live region: {live_text:?}"
    );

    let text = copy_text(&v);
    assert!(
        !text.contains("the answer"),
        "an open Say never reaches the transcript copy text: {text}"
    );
    assert!(
        !text.contains("running "),
        "no live spinner hint leaks into copy text: {text}"
    );
}

#[test]
fn running_call_header_lives_only_in_the_live_region_and_never_leaks_into_copy_text() {
    // A call whose `ToolEnd` has not arrived renders its header with a
    // trailing `" ⠋"` motion span (chat_step_render: `elapsed_ms` unset).
    // While the turn runs that row lives ONLY in the TURN LIVE REGION (its
    // own area — the transcript holds just the folded ladder row), so copy
    // mode over the transcript can never pick up the spinner glyph.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("thinking".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "t1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    // Precondition: the live region really carries the spinner span on the
    // running call's header row (tick 0 → first spinner frame).
    let live_rows = crate::chat::live_turn::live_data(&v.blocks, v.turn_block_start)
        .map(|live| crate::chat::live_turn::render_lines(live.steps, live.say, 0))
        .unwrap_or_default();
    let live_text = live_rows
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        live_text.contains('\u{280b}') && live_text.contains("bash"),
        "precondition: the live region's running call header carries the \
spinner glyph: {live_text:?}"
    );
    assert!(
        !row_texts(&v).iter().any(|l| l.contains("bash")),
        "the transcript holds only the folded ladder while the region streams"
    );

    let text = copy_text(&v);
    assert!(
        !text.contains('\u{280b}') && !text.contains("bash echo x"),
        "copy text stays transcript-only: no spinner, no live-region rows: {text}"
    );
}

#[test]
fn expanded_ladder_keeps_thinking_calls_and_full_say() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("deep thinking".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta(
        "the final answer\nsecond line".into(),
    ));
    v.apply(&SessionEvent::Done);

    // Open the whole ladder: pair → step → calls aggregate → call result
    // (the fixed-index walk `chat_tests::say_pair` uses).
    v.toggle_tool_call_at(0, 0);
    assert!(
        row_texts(&v).iter().any(|l| l.contains("Step(1)")),
        "precondition: the first toggle opens the ladder's step rows"
    );
    v.toggle_tool_call_at(0, 1);
    v.toggle_tool_call_at(0, 2);
    v.toggle_tool_call_at(0, 3);

    let text = copy_text(&v);
    assert!(
        text.contains("deep thinking"),
        "expanded step-thinking body stays visible in copy mode: {text}"
    );
    assert!(
        text.contains("t1-out"),
        "expanded call output stays visible in copy mode: {text}"
    );
    assert!(
        text.contains("the final answer") && text.contains("second line"),
        "the full Say survives alongside the expanded ladder: {text}"
    );
}
