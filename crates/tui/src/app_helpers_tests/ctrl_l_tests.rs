//! Subagent-exit + follow-mode intercept tests (Esc / Ctrl+L in
//! `pre_key_intercept`). Split from `mod.rs` to keep files within the
//! iteration caps.

use crate::app_helpers::pre_key_intercept;
use crate::chat::{ChatBlock, ChatView};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_session::SessionEvent;

/// Build a parent view with a collapsible thinking block and one Subagent
/// block whose child view also has an (expanded) thinking block.
fn chat_with_subagent() -> (ChatView, usize) {
    let mut chat = ChatView::default();
    chat.apply(&SessionEvent::ReasoningDelta("parent-reason".into()));
    chat.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "find it".into(),
        child_session_id: "c1".into(),
    });
    let sub_idx = chat
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::Subagent { .. }))
        .expect("a Subagent block exists");
    // Give the child view a collapsible thinking block and expand it, so the
    // collapse-on-exit is observable.
    if let ChatBlock::Subagent { view, .. } = &mut chat.blocks[sub_idx] {
        view.apply(&SessionEvent::ReasoningDelta("child-reason".into()));
        for h in view.thinking_headers() {
            view.toggle_thinking_at(h.block_idx);
        }
    }
    // Expand the parent's thinking block too.
    for h in chat.thinking_headers() {
        chat.toggle_thinking_at(h.block_idx);
    }
    (chat, sub_idx)
}

fn assert_all_thinking_collapsed(chat: &ChatView) {
    for b in &chat.blocks {
        if let ChatBlock::Thinking { collapsed, .. } = b {
            assert!(*collapsed, "thinking block must be collapsed");
        }
    }
}

/// Ctrl+L inside a focused subagent view collapses the CHILD's blocks, exits
/// back to the parent view, collapses the PARENT's blocks, clears the input,
/// and returns to FOLLOW MODE (bottom of the view) — the newest content is
/// always in view after the reset.
#[test]
fn ctrl_l_exits_subagent_and_returns_to_follow_mode() {
    let (mut chat, sub_idx) = chat_with_subagent();

    let mut subagent_focus = Some(sub_idx);
    let mut follow = false;
    let mut selection = None;
    let mut last_esc = None;
    let mut input = "hello".to_string();
    let mut cursor = 5usize;
    let mut needs_clear = false;
    let consumed = pre_key_intercept(
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
        &mut subagent_focus,
        &mut follow,
        &mut selection,
        &mut last_esc,
        &mut chat,
        &mut input,
        &mut cursor,
        &mut needs_clear,
    );

    assert!(consumed, "Ctrl+L must be consumed by pre_key_intercept");
    assert_eq!(
        subagent_focus, None,
        "Ctrl+L must exit the focused subagent view"
    );
    assert!(follow, "Ctrl+L must return to follow mode (bottom of view)");
    assert!(input.is_empty(), "Ctrl+L must clear the input");
    assert_eq!(cursor, 0, "Ctrl+L must reset the cursor");
    assert!(!needs_clear, "Ctrl+L must not force a full-screen redraw");

    // Child view: its thinking block was collapsed before exiting.
    if let ChatBlock::Subagent { view, .. } = &chat.blocks[sub_idx] {
        let all_collapsed = view
            .blocks
            .iter()
            .filter_map(|b| match b {
                ChatBlock::Thinking { collapsed, .. } => Some(*collapsed),
                _ => None,
            })
            .all(|c| c);
        assert!(
            all_collapsed,
            "child thinking block must be collapsed on exit"
        );
    }
    // Parent view: its thinking block was collapsed too.
    assert_all_thinking_collapsed(&chat);
}

/// Esc exits a focused subagent view back to the parent at FOLLOW MODE
/// (bottom of the view) — same reset-to-live semantics as Ctrl+L, without
/// the collapse / input-clear side effects.
#[test]
fn esc_exits_subagent_and_returns_to_follow_mode() {
    let (mut chat, sub_idx) = chat_with_subagent();

    let mut subagent_focus = Some(sub_idx);
    let mut follow = false;
    let mut selection = None;
    let mut last_esc = None;
    let mut input = "draft".to_string();
    let mut cursor = 2usize;
    let mut needs_clear = false;
    let consumed = pre_key_intercept(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &mut subagent_focus,
        &mut follow,
        &mut selection,
        &mut last_esc,
        &mut chat,
        &mut input,
        &mut cursor,
        &mut needs_clear,
    );

    assert!(consumed, "Esc must be consumed by pre_key_intercept");
    assert_eq!(subagent_focus, None, "Esc must exit the subagent view");
    assert!(follow, "Esc must return to follow mode (bottom of view)");
    // Esc is exit-only: no collapse, no input clear, no redraw.
    assert_eq!(input, "draft", "Esc must not clear the input");
    assert_eq!(cursor, 2, "Esc must not move the cursor");
    assert!(!needs_clear, "Esc must not force a full-screen redraw");
    // Parent blocks are NOT collapsed by Esc (Ctrl+L owns collapse).
    for b in &chat.blocks {
        if let ChatBlock::Thinking { collapsed, .. } = b {
            assert!(!*collapsed, "Esc must not collapse thinking blocks");
        }
    }
}

/// Ctrl+L with NO subagent focused still collapses blocks, clears the input
/// and returns to follow mode (so a scrolled-up parent also jumps to bottom).
#[test]
fn ctrl_l_without_subagent_returns_to_follow_mode() {
    let mut chat = ChatView::default();
    chat.apply(&SessionEvent::ReasoningDelta("parent-reason".into()));

    let mut subagent_focus = None;
    let mut follow = false;
    let mut selection = None;
    let mut last_esc = None;
    let mut input = "draft".to_string();
    let mut cursor = 3usize;
    let mut needs_clear = false;
    let consumed = pre_key_intercept(
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
        &mut subagent_focus,
        &mut follow,
        &mut selection,
        &mut last_esc,
        &mut chat,
        &mut input,
        &mut cursor,
        &mut needs_clear,
    );

    assert!(consumed, "Ctrl+L must be consumed by pre_key_intercept");
    assert_eq!(subagent_focus, None);
    assert!(
        follow,
        "Ctrl+L must return to follow mode even without a subagent"
    );
    assert!(input.is_empty(), "Ctrl+L must clear the input");
    assert_eq!(cursor, 0, "Ctrl+L must reset the cursor");
    assert_all_thinking_collapsed(&chat);
}
