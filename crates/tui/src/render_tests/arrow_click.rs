use super::*;
use crate::chat::ChatView;
use opencoder_session::SessionEvent;

/// Regression for "arrow buttons do nothing, especially when idle". The
/// top-level `render()` must export clickable arrow targets into `MouseHits`:
///   - jump-to-bottom (visible whenever the view is NOT following the bottom),
///   - jump-to-top   (visible whenever the body is scrolled past the top).
/// The event loop forwards these rects straight to `handle_mouse`; if they are
/// absent the arrows silently do nothing. Originally the loop reset `hits` to
/// `default()` every iteration, so in idle state (no render) every click was
/// lost. This exercises the full render->mouse path on a `TestBackend`.
#[tokio::test]
async fn render_then_click_arrow_targets_jump_view() {
    use crate::app_helpers::{handle_mouse, MouseOutcome};
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use opencoder_store::LibsqlStore;
    use std::sync::Arc;

    // Tall chat so the body scrolls (max_rows > 0) and both arrows can appear.
    let mut chat = ChatView::default();
    for i in 0..80 {
        chat.apply(&SessionEvent::TextDelta(format!(
            "content line number {i}\n"
        )));
    }
    chat.apply(&SessionEvent::Done);

    let store: Arc<dyn opencoder_store::Store> =
        Arc::new(LibsqlStore::open_memory().await.unwrap());

    // --- jump-to-bottom arrow: rendered only when follow == false ---
    {
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let mut scroll = 0u32;
        let mut queue_scroll: u32 = 0;
        let mut hits = MouseHits::default();
        render(
            &mut terminal,
            &chat,
            "",
            0,
            &Line::raw("title"),
            false,
            0,
            0,
            200_000,
            200_000,
            "idle",
            &[],
            &[],
            &mut scroll,
            false, // not following -> arrow visible
            &mut queue_scroll,
            0,
            0,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &mut hits,
            &mut None,
            None,
            None,
            &[],
            false,
            None,
            0,
            0,
            true,
            false,
            "act",
        )
        .unwrap();
        let btn = hits
            .jump_btn
            .expect("render must export jump-bottom arrow when not following");

        let mut follow = false;
        let mut sel: Option<(u32, u32)> = None;
        let mut subagent_focus: Option<usize> = None;
        let mut subagent_sys = 0u64;
        let mut queue_items: Vec<(i64, String)> = Vec::new();
        let mut copy_msg: Option<String> = None;
        let mut last_click: Option<std::time::Instant> = None;
        let mut dbl_click = false;
        let mut queue_scroll: u32 = 0;
        let outcome = handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: btn.x,
                row: btn.y,
                modifiers: KeyModifiers::NONE,
            },
            &hits,
            &mut scroll,
            &mut follow,
            &mut sel,
            &mut chat,
            &mut subagent_focus,
            &mut subagent_sys,
            std::path::Path::new("."),
            &mut queue_items,
            "s1",
            store.as_ref(),
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
            &mut queue_scroll,
        )
        .await;
        assert_eq!(outcome, MouseOutcome::None);
        assert!(follow, "clicking the jump-bottom arrow must enable follow");
    }

    // --- jump-to-top arrow: rendered only when scrolled past the top ---
    {
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        // follow=false keeps the supplied scroll (clamped to max_rows).
        let mut scroll = 30u32;
        let mut queue_scroll: u32 = 0;
        let mut hits = MouseHits::default();
        render(
            &mut terminal,
            &chat,
            "",
            0,
            &Line::raw("title"),
            false,
            0,
            0,
            200_000,
            200_000,
            "idle",
            &[],
            &[],
            &mut scroll,
            false,
            &mut queue_scroll,
            0,
            0,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &mut hits,
            &mut None,
            None,
            None,
            &[],
            false,
            None,
            0,
            0,
            true,
            false,
            "act",
        )
        .unwrap();
        assert!(scroll > 0, "precondition: body must be scrolled");
        let btn = hits
            .top_btn
            .expect("render must export jump-top arrow when scrolled");

        let mut follow = true;
        let mut sel: Option<(u32, u32)> = None;
        let mut subagent_focus: Option<usize> = None;
        let mut subagent_sys = 0u64;
        let mut queue_items: Vec<(i64, String)> = Vec::new();
        let mut copy_msg: Option<String> = None;
        let mut last_click: Option<std::time::Instant> = None;
        let mut dbl_click = false;
        let mut queue_scroll: u32 = 0;
        let outcome = handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: btn.x,
                row: btn.y,
                modifiers: KeyModifiers::NONE,
            },
            &hits,
            &mut scroll,
            &mut follow,
            &mut sel,
            &mut chat,
            &mut subagent_focus,
            &mut subagent_sys,
            std::path::Path::new("."),
            &mut queue_items,
            "s1",
            store.as_ref(),
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
            &mut queue_scroll,
        )
        .await;
        assert_eq!(outcome, MouseOutcome::None);
        assert_eq!(scroll, 0, "clicking the jump-top arrow must reset scroll");
        assert!(!follow, "clicking the jump-top arrow must disable follow");
    }
}
