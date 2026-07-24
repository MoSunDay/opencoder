//! Regression tests for the mouse wheel-scroll handler, focusing on the
//! bug where `ScrollDown` computed `max_rows` from the PARENT chat even
//! while a subagent perspective was focused — pinning to the bottom and
//! making the child body un-scrollable.
use super::*;
use async_trait::async_trait;
use opencoder_core::Message;
use opencoder_session::SessionEvent;
use opencoder_store::{
    SessionEventRecord, SessionFilter, SessionListItem, SessionMeta, SessionPatch,
    SubagentTaskRecord,
};
use ratatui::layout::Rect;

#[test]
fn paste_existing_absolute_file_echoes_full_path() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let raw = tmp.path().to_string_lossy().into_owned();
    let expected = tmp
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    // Absolute paths ignore workdir.
    assert_eq!(paste_payload(&raw, Path::new("/")), expected);
}

#[test]
fn paste_existing_file_with_trailing_newline_echoes_full_path() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let raw = tmp.path().to_string_lossy().into_owned();
    let expected = tmp
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(paste_payload(&format!("{raw}\n"), Path::new("/")), expected);
}

#[test]
fn paste_quoted_absolute_file_echoes_full_path() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let raw = tmp.path().to_string_lossy().into_owned();
    let expected = tmp
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(paste_payload(&format!("'{raw}'"), Path::new("/")), expected);
    assert_eq!(paste_payload(&format!("{raw}\""), Path::new("/")), expected);
}

#[test]
fn paste_existing_relative_file_resolves_against_workdir() {
    // A drag-pasted bare relative filename resolves to its full absolute
    // path when it exists relative to the session workdir.
    let dir = tempfile::tempdir().unwrap();
    let rel = "src/main.rs";
    let abs = dir.path().join(rel);
    std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
    std::fs::write(&abs, "fn main(){}").unwrap();
    let expected = abs.canonicalize().unwrap().to_string_lossy().into_owned();
    assert_eq!(paste_payload(rel, dir.path()), expected);
}

#[test]
fn paste_nonexistent_absolute_path_returned_verbatim() {
    let raw = "/this/does/not/exist/xyz";
    assert_eq!(paste_payload(raw, Path::new("/")), raw);
}

#[test]
fn paste_multiline_text_returned_verbatim() {
    let raw = "first line\nsecond line\n";
    assert_eq!(paste_payload(raw, Path::new("/")), raw);
}

#[test]
fn paste_empty_returned_verbatim() {
    assert_eq!(paste_payload("", Path::new("/")), "");
    assert_eq!(paste_payload("\n", Path::new("/")), "\n");
}

#[test]
fn paste_non_file_text_returned_verbatim() {
    // A plain word that is not an existing file relative to workdir is
    // never rewritten, so ordinary text pastes are never surprising.
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(paste_payload("hello world", dir.path()), "hello world");
}

/// `Store` whose every method panics. The `ScrollDown` branch of
/// `handle_mouse` never touches the store, so passing a reference is safe;
/// if a method were ever invoked it would fail loudly.
struct StubStore;

#[async_trait]
impl Store for StubStore {
    fn backend_name(&self) -> &'static str {
        "stub"
    }
    async fn create_session(&self, _: &SessionMeta) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn get_session(&self, _: &str) -> anyhow::Result<Option<SessionMeta>> {
        unimplemented!()
    }
    async fn list_sessions(&self, _: &SessionFilter) -> anyhow::Result<Vec<SessionListItem>> {
        unimplemented!()
    }
    async fn update_session(&self, _: &str, _: &SessionPatch) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn delete_session(&self, _: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn clear_other_sessions(&self, _: &str) -> anyhow::Result<u64> {
        unimplemented!()
    }
    async fn append_message(&self, _: &str, _: &Message) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn append_messages(&self, _: &str, _: &[Message]) -> anyhow::Result<Vec<i64>> {
        unimplemented!()
    }
    async fn load_messages(&self, _: &str) -> anyhow::Result<Vec<Message>> {
        unimplemented!()
    }
    async fn last_message_seq(&self, _: &str) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn admit_input(&self, _: &SessionInput) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn pending_inputs(&self, _: &str, _: Delivery) -> anyhow::Result<Vec<SessionInput>> {
        unimplemented!()
    }
    async fn promote_inputs(&self, _: &str, _: i64, _: Delivery) -> anyhow::Result<Vec<i64>> {
        unimplemented!()
    }
    async fn promote_next_queued(&self, _: &str) -> anyhow::Result<Option<i64>> {
        unimplemented!()
    }
    async fn claim_next_queue(&self, _: &str) -> anyhow::Result<Option<(i64, SessionInput)>> {
        unimplemented!()
    }
    async fn delete_input(&self, _: i64) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn swap_input_order(&self, _: &str, _: i64, _: i64) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn append_events(&self, _: &[SessionEventRecord]) -> anyhow::Result<Vec<i64>> {
        unimplemented!()
    }
    async fn events_after(&self, _: &str, _: i64) -> anyhow::Result<Vec<SessionEventRecord>> {
        unimplemented!()
    }
    async fn last_event_seq(&self, _: &str) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn create_subagent_task(&self, _: &SubagentTaskRecord) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn complete_subagent_task(&self, _: &str, _: &str, _: bool) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn list_subagent_tasks(&self, _: &str) -> anyhow::Result<Vec<SubagentTaskRecord>> {
        unimplemented!()
    }
    async fn get_subagent_task(&self, _: &str) -> anyhow::Result<Option<SubagentTaskRecord>> {
        unimplemented!()
    }
    async fn cancel_subagent_task(&self, _: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
}

/// Parent whose own content is short but wraps a subagent whose CHILD view
/// is long. Left unfinalized so `flatten` emits the raw lines verbatim
/// (row count independent of the markdown renderer).
fn parent_with_long_subagent() -> ChatView {
    let mut chat = ChatView::default();
    chat.apply(&SessionEvent::TextDelta("parent preamble".into()));
    chat.apply(&SessionEvent::Done);
    chat.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "find it".into(),
        child_session_id: "c1".into(),
    });
    let child_text = (0..40)
        .map(|i| format!("child output line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    chat.apply(&SessionEvent::SubagentChild {
        id: "s1".into(),
        ev: Box::new(SessionEvent::TextDelta(child_text)),
    });
    chat
}

fn empty_hits(body: Rect) -> MouseHits {
    MouseHits {
        jump_btn: None,
        top_btn: None,
        body: Some(body),
        queue_btns: Vec::new(),
        thinking_btns: Vec::new(),
        subagent_btns: Vec::new(),
    }
}

fn scroll_down() -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 40,
        row: 6,
        modifiers: KeyModifiers::NONE,
    }
}

/// The regression: with a subagent focused, one wheel-down must NOT pin to
/// the bottom even though the PARENT fits in the viewport (which, under the
/// old parent-based `max_rows`, saturated to 0 and tripped `follow`).
#[tokio::test]
async fn scrolldown_in_subagent_view_uses_child_content() {
    let mut chat = parent_with_long_subagent();
    let sub_idx = chat
        .blocks
        .iter()
        .position(|b| matches!(b, crate::chat::ChatBlock::Subagent { .. }))
        .expect("a Subagent block exists");

    let parent_rows = chat.flatten().len();
    let child_rows = match &chat.blocks[sub_idx] {
        crate::chat::ChatBlock::Subagent { view, .. } => view.flatten().len(),
        _ => unreachable!(),
    };
    let body = Rect::new(0, 0, 80, 12); // visible_h = 10, inner_w = 77
    let visible_h = body.height as usize - 2;
    assert!(
            child_rows > parent_rows && child_rows > visible_h,
            "precondition: child ({child_rows}) longer than parent ({parent_rows}) and viewport ({visible_h})"
        );
    // Parent must fit in the viewport — that is what made the old math trip.
    assert!(
        parent_rows < visible_h,
        "precondition: parent ({parent_rows}) fits viewport ({visible_h})"
    );

    let hits = empty_hits(body);
    let mut scroll = 0u16;
    let mut follow = false;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus = Some(sub_idx);
    let mut parent_scroll = 0u16;
    let mut parent_follow = false;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore;
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;

    handle_mouse(
        scroll_down(),
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut parent_scroll,
        &mut parent_follow,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
    )
    .await;

    assert_eq!(scroll, 3, "scroll advanced by one notch");
    assert!(
        !follow,
        "follow must NOT trip: the child still has content below the fold"
    );
}

/// Mirror case: with NO subagent focused, the parent view drives `max_rows`.
/// Here the short parent fits the viewport, so the first wheel-down
/// legitimately pins to the bottom.
#[tokio::test]
async fn scrolldown_uses_parent_when_no_subagent_focused() {
    let mut chat = parent_with_long_subagent();
    let body = Rect::new(0, 0, 80, 12);
    let visible_h = body.height as usize - 2;
    assert!(
        chat.flatten().len() < visible_h,
        "precondition: parent fits viewport"
    );

    let hits = empty_hits(body);
    let mut scroll = 0u16;
    let mut follow = false;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut parent_scroll = 0u16;
    let mut parent_follow = false;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore;
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;

    handle_mouse(
        scroll_down(),
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut parent_scroll,
        &mut parent_follow,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
    )
    .await;

    assert!(
        follow,
        "short parent legitimately pins to bottom immediately"
    );
}

#[tokio::test]
async fn dbl_click_selects_line_and_copies_on_release() {
    // Build a chat view with 5 marker lines (abs rows 0-4).
    let mut chat = ChatView::default();
    for &l in &[
        "line one",
        "line two",
        "line three",
        "line four",
        "line five",
    ] {
        chat.push_marker(Line::from(l.to_string()));
    }

    // Body rect: inner_y=1, inner_h=10, so screen row 5 maps to abs row 4.
    let body = Rect::new(0, 0, 80, 12);
    let hits = empty_hits(body);

    let mut scroll = 0u16;
    let mut follow = true;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut parent_scroll = 0u16;
    let mut parent_follow = true;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;
    let store = StubStore;

    let mk_down = |row| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row,
        modifiers: KeyModifiers::NONE,
    };

    // First click — should NOT set dbl_click.
    handle_mouse(
        mk_down(5),
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut parent_scroll,
        &mut parent_follow,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
    )
    .await;
    assert!(!dbl_click, "first click should not be a double-click");

    // Second click immediately — should set dbl_click and selection.
    handle_mouse(
        mk_down(5),
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut parent_scroll,
        &mut parent_follow,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
    )
    .await;
    assert!(dbl_click, "second click should be detected as double-click");
    assert!(selection.is_some(), "selection should be set on dbl-click");

    // Mouse up — should copy (force=true via dbl_click).
    let up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 10,
        row: 5,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        up,
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut parent_scroll,
        &mut parent_follow,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
    )
    .await;
    assert!(copy_msg.is_some(), "double-click should copy on release");
    assert!(selection.is_none(), "selection cleared after release");
    assert!(!dbl_click, "dbl_click reset after release");
}

#[tokio::test]
async fn submit_btn_returns_steer_submit() {
    let mut chat = ChatView::default();
    let body = Rect::new(0, 0, 80, 12);

    // Build a MouseHits with a Submit button for steer seq=10 at (77, 0).
    let mut hits = empty_hits(body);
    hits.queue_btns.push(queue_panel::QueueBtn {
        seq: 10,
        action: queue_panel::QueueBtnAction::Submit,
        rect: Rect::new(77, 0, 1, 1),
    });

    let mut scroll = 0u16;
    let mut follow = true;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut parent_scroll = 0u16;
    let mut parent_follow = true;
    let mut subagent_sys = 0u64;
    chat.steer_items = vec![(10, "redirect".into())];
    let mut queue_items: Vec<(i64, String)> = vec![];
    let store = StubStore;
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;

    let down = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 77,
        row: 0,
        modifiers: KeyModifiers::NONE,
    };
    let outcome = handle_mouse(
        down,
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut parent_scroll,
        &mut parent_follow,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
    )
    .await;

    assert_eq!(
        outcome,
        MouseOutcome::SteerSubmit,
        "clicking Submit on a steer row must return SteerSubmit"
    );
    // Steer item must NOT be removed — promotion happens in the drain loop.
    assert_eq!(
        chat.steer_items.len(),
        1,
        "steer item should remain until drain"
    );
}

#[tokio::test]
async fn single_click_does_not_copy_on_release() {
    let mut chat = ChatView::default();
    for &l in &[
        "line one",
        "line two",
        "line three",
        "line four",
        "line five",
    ] {
        chat.push_marker(Line::from(l.to_string()));
    }

    let body = Rect::new(0, 0, 80, 12);
    let hits = empty_hits(body);

    let mut scroll = 0u16;
    let mut follow = true;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut parent_scroll = 0u16;
    let mut parent_follow = true;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;
    let store = StubStore;

    let down = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row: 5,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        down,
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut parent_scroll,
        &mut parent_follow,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
    )
    .await;
    assert!(!dbl_click);

    let up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 10,
        row: 5,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        up,
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut parent_scroll,
        &mut parent_follow,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
    )
    .await;
    assert!(copy_msg.is_none(), "single click should not copy");
}

/// Regression: clicking the follow/jump button immediately after a body
/// click must still work. Previously the `jump_btn` check sat AFTER the
/// double-click guard, so the second click (within 400 ms) was swallowed
/// by `is_dbl` and the early `return`, making the follow button
/// unreliable.
#[tokio::test]
async fn jump_btn_click_works_after_recent_body_click() {
    let mut chat = ChatView::default();
    chat.push_marker(Line::from("some text"));

    let body = Rect::new(0, 0, 80, 12);
    // jump_btn sits on the body's bottom-border row, right-aligned.
    let jump_btn_rect = Rect::new(74, 11, 6, 1);
    let hits = MouseHits {
        jump_btn: Some(jump_btn_rect),
        top_btn: None,
        body: Some(body),
        queue_btns: Vec::new(),
        thinking_btns: Vec::new(),
        subagent_btns: Vec::new(),
    };

    let mut scroll = 0u16;
    let mut follow = false;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut parent_scroll = 0u16;
    let mut parent_follow = false;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;
    let store = StubStore;

    // First click: hits the body interior (row 5, well inside body).
    let body_click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row: 5,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        body_click,
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut parent_scroll,
        &mut parent_follow,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
    )
    .await;
    assert!(last_click.is_some(), "body click should set last_click");
    assert!(!follow, "body click should not set follow");

    // Second click immediately after (< 400 ms): hits the jump button.
    // Under the old code this was swallowed by the double-click guard.
    let jump_click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 76,
        row: 11,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        jump_click,
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut parent_scroll,
        &mut parent_follow,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
    )
    .await;
    assert!(
        follow,
        "jump button click must set follow=true even right after a body click"
    );
}

/// Wheel-up now advances 8 lines per notch (was 3) so scrolling back up
/// through a long transcript feels responsive. Down is unchanged at 3.
#[tokio::test]
async fn scrollup_advances_faster_than_default() {
    // Build a long-enough ChatView so content clearly exceeds the small
    // viewport (visible_h = body.height - 2 = 10).
    let mut chat = ChatView::default();
    for n in 0..30u32 {
        chat.push_marker(Line::from(format!("marker line {n}")));
    }

    let body = Rect::new(0, 0, 80, 12);
    let hits = empty_hits(body);

    let scroll_up = || MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 40,
        row: 6,
        modifiers: KeyModifiers::NONE,
    };

    // `scroll` is the top-anchored line offset (0 == top); scroll-up moves
    // toward the top via `saturating_sub`. Start part-way down so a single
    // notch lands on a value that proves the 8-line step: the new step
    // yields 16 - 8 = 8, whereas the old 3-step would have left 16 - 3 = 13.
    let mut scroll = 16u16;
    let mut follow = true;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut parent_scroll = 0u16;
    let mut parent_follow = false;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;
    let store = StubStore;

    handle_mouse(
        scroll_up(),
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut parent_scroll,
        &mut parent_follow,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
    )
    .await;

    assert_eq!(scroll, 8, "one wheel-up notch now moves 8 lines (was 3)");
    assert!(!follow, "scrolling up must detach from the tail");
}

/// Regression: clicking a Thinking-block header must toggle on the FIRST
/// click even when it lands within the 400 ms double-click window of a
/// previous click. Previously the thinking-toggle loop sat AFTER the
/// dbl-click guard, so any header click within 400 ms of a prior click was
/// swallowed by the guard's early `return` (selecting a line instead) and
/// the toggle never ran — making expansion probabilistic. The fix moves
/// queue/thinking/subagent button-hit detection ahead of the guard, the
/// same fix jump_btn/top_btn already had.
#[tokio::test]
async fn thinking_header_toggles_even_right_after_another_click() {
    let mut chat = ChatView::default();
    chat.apply(&SessionEvent::ReasoningDelta(
        "secret reasoning here".into(),
    ));
    chat.apply(&SessionEvent::TextDelta("answer".into()));
    chat.apply(&SessionEvent::Done);
    // Collapsed by default: the reasoning content must NOT be visible yet.
    assert!(
        !chat.flatten().iter().any(|l| l
            .spans
            .iter()
            .any(|s| s.content.contains("secret reasoning"))),
        "precondition: thinking must start collapsed"
    );

    let body = Rect::new(0, 0, 80, 12);
    let header_rect = Rect::new(1, 1, 78, 1);
    let hits = MouseHits {
        jump_btn: None,
        top_btn: None,
        body: Some(body),
        queue_btns: Vec::new(),
        thinking_btns: vec![crate::render::ThinkingBtn {
            block_idx: 0,
            rect: header_rect,
        }],
        subagent_btns: Vec::new(),
    };

    let mut scroll = 0u16;
    let mut follow = false;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut parent_scroll = 0u16;
    let mut parent_follow = false;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore;
    let mut copy_msg: Option<String> = None;
    // A click ~50 ms ago — squarely inside the 400 ms dbl-click window.
    // On the buggy code this trips `is_dbl` and the toggle is skipped.
    let mut last_click: Option<Instant> = Some(Instant::now());
    let mut dbl_click = false;

    let outcome = handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: header_rect.x,
            row: header_rect.y,
            modifiers: KeyModifiers::NONE,
        },
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut parent_scroll,
        &mut parent_follow,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
    )
    .await;
    assert_eq!(outcome, MouseOutcome::None);
    assert!(
        chat.flatten().iter().any(|l| l
            .spans
            .iter()
            .any(|s| s.content.contains("secret reasoning"))),
        "thinking must be expanded after the header click"
    );
    assert!(
        !dbl_click,
        "a header toggle must not be flagged as a double-click"
    );
}

#[tokio::test]
async fn clear_pending_inputs_drops_store_rows_and_mirrors() {
    use opencoder_store::LibsqlStore;
    let store = LibsqlStore::open_memory().await.unwrap();
    let sid = "s1";
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let s_seq = store
        .admit_input(&mk_input(sid, Delivery::Steer, "steer-1"))
        .await
        .unwrap();
    let q_seq = store
        .admit_input(&mk_input(sid, Delivery::Queue, "queue-1"))
        .await
        .unwrap();
    let mut steer_items = vec![(s_seq, String::from("steer-1"))];
    let mut queue_items = vec![(q_seq, String::from("queue-1"))];

    clear_pending_inputs(&store, &mut steer_items, &mut queue_items).await;

    assert!(steer_items.is_empty(), "steer mirror cleared");
    assert!(queue_items.is_empty(), "queue mirror cleared");
    assert!(
        store
            .pending_inputs(sid, Delivery::Steer)
            .await
            .unwrap()
            .is_empty(),
        "steer rows deleted from store"
    );
    assert!(
        store
            .pending_inputs(sid, Delivery::Queue)
            .await
            .unwrap()
            .is_empty(),
        "queue rows deleted from store"
    );
}

/// Ctrl+U must behave identically to Ctrl+L: consume the key, clear the input
/// box, and reset the cursor. Regression guard for the keybinding unification.
#[test]
fn ctrl_u_matches_ctrl_l_clears_input() {
    fn run(key: KeyEvent) -> (bool, String, usize) {
        let mut chat = ChatView::default();
        let mut subagent_focus: Option<usize> = None;
        let mut scroll = 5u16;
        let mut follow = true;
        let mut selection = None;
        let mut last_esc = None;
        let mut input = "hello world".to_string();
        let mut cursor = 5usize;
        let consumed = pre_key_intercept(
            key,
            &mut subagent_focus,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut last_esc,
            &mut chat,
            &mut input,
            &mut cursor,
            0,
            true,
        );
        (consumed, input, cursor)
    }

    let ctrl_u = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);
    let ctrl_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);

    let (u_consumed, u_input, u_cursor) = run(ctrl_u);
    let (l_consumed, l_input, l_cursor) = run(ctrl_l);

    assert!(u_consumed, "Ctrl+U must be consumed by pre_key_intercept");
    assert!(u_input.is_empty(), "Ctrl+U must clear the input");
    assert_eq!(u_cursor, 0, "Ctrl+U must reset the cursor");
    // Identical outcome to Ctrl+L.
    assert_eq!((u_consumed, u_input.as_str(), u_cursor), (l_consumed, l_input.as_str(), l_cursor));
}
