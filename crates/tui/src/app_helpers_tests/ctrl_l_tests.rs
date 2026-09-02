//! Subagent-exit + follow-mode intercept tests (Esc / Ctrl+L in
//! `pre_key_intercept`). Split from `mod.rs` to keep files within the
//! iteration caps.

use crate::app_helpers::pre_key_intercept;
use crate::chat::{ChatBlock, ChatView};
use crate::keymap::KeyBindings;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_session::SessionEvent;
use tokio::sync::mpsc;

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
    let mut last_esc = None;
    let mut input = "hello".to_string();
    let mut cursor = 5usize;
    let mut needs_clear = false;
    let (sidecar_tx, _sidecar_rx) = mpsc::channel::<crate::sidecar_ui::SidecarCmd>(8);
    let consumed = pre_key_intercept(
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
        &KeyBindings::default(),
        &mut subagent_focus,
        &mut follow,
        &mut last_esc,
        &mut chat,
        &mut input,
        &mut cursor,
        &mut needs_clear,
        &sidecar_tx,
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

/// Ctrl+L resets every ToolGroup (parent AND focused child) to the Collapsed
/// state — expanded List/Results groups are one keystroke away from the
/// single count line.
#[test]
fn ctrl_l_resets_tool_groups_to_collapsed() {
    use opencoder_session::SessionEvent;

    let (mut chat, sub_idx) = chat_with_subagent();
    // Give both views a finished tool group and expand it to Results.
    chat.apply(&SessionEvent::ToolStart {
        id: "p".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "ls"}),
    });
    chat.apply(&SessionEvent::ToolEnd {
        id: "p".into(),
        name: "bash".into(),
        output: "out".into(),
        is_error: false,
        images: Vec::new(),
    });
    if let ChatBlock::Subagent { view, .. } = &mut chat.blocks[sub_idx] {
        view.apply(&SessionEvent::ToolStart {
            id: "c".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "ls"}),
        });
        view.apply(&SessionEvent::ToolEnd {
            id: "c".into(),
            name: "bash".into(),
            output: "out".into(),
            is_error: false,
            images: Vec::new(),
        });
        for h in view.tool_headers() {
            view.cycle_tool_group_at(h.block_idx);
            view.cycle_tool_group_at(h.block_idx); // -> Results
        }
    }
    for h in chat.tool_headers() {
        chat.cycle_tool_group_at(h.block_idx);
        chat.cycle_tool_group_at(h.block_idx); // -> Results
    }

    let mut subagent_focus = Some(sub_idx);
    let mut follow = false;
    let mut last_esc = None;
    let mut input = String::new();
    let mut cursor = 0usize;
    let mut needs_clear = false;
    let (sidecar_tx, _sidecar_rx) = mpsc::channel::<crate::sidecar_ui::SidecarCmd>(8);
    let consumed = pre_key_intercept(
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
        &KeyBindings::default(),
        &mut subagent_focus,
        &mut follow,
        &mut last_esc,
        &mut chat,
        &mut input,
        &mut cursor,
        &mut needs_clear,
        &sidecar_tx,
    );
    assert!(consumed, "Ctrl+L must be consumed");

    let all_collapsed = |v: &ChatView| {
        v.blocks.iter().all(|b| {
            !matches!(
                b,
                ChatBlock::ToolGroup {
                    state: crate::chat::ToolGroupState::List,
                    ..
                }
            ) && !matches!(
                b,
                ChatBlock::ToolGroup {
                    state: crate::chat::ToolGroupState::Results,
                    ..
                }
            )
        })
    };
    assert!(
        all_collapsed(&chat),
        "parent tool groups must be Collapsed after Ctrl+L"
    );
    if let ChatBlock::Subagent { view, .. } = &chat.blocks[sub_idx] {
        assert!(
            all_collapsed(view),
            "child tool groups must be Collapsed after Ctrl+L"
        );
    }
}

/// Esc exits a focused subagent view back to the parent at FOLLOW MODE
/// (bottom of the view) — same reset-to-live semantics as Ctrl+L, without
/// the collapse / input-clear side effects.
#[test]
fn esc_exits_subagent_and_returns_to_follow_mode() {
    let (mut chat, sub_idx) = chat_with_subagent();

    let mut subagent_focus = Some(sub_idx);
    let mut follow = false;
    let mut last_esc = None;
    let mut input = "draft".to_string();
    let mut cursor = 2usize;
    let mut needs_clear = false;
    let (sidecar_tx, _sidecar_rx) = mpsc::channel::<crate::sidecar_ui::SidecarCmd>(8);
    let consumed = pre_key_intercept(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &KeyBindings::default(),
        &mut subagent_focus,
        &mut follow,
        &mut last_esc,
        &mut chat,
        &mut input,
        &mut cursor,
        &mut needs_clear,
        &sidecar_tx,
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
    let mut last_esc = None;
    let mut input = "draft".to_string();
    let mut cursor = 3usize;
    let mut needs_clear = false;
    let (sidecar_tx, _sidecar_rx) = mpsc::channel::<crate::sidecar_ui::SidecarCmd>(8);
    let consumed = pre_key_intercept(
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
        &KeyBindings::default(),
        &mut subagent_focus,
        &mut follow,
        &mut last_esc,
        &mut chat,
        &mut input,
        &mut cursor,
        &mut needs_clear,
        &sidecar_tx,
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

/// ESC on a focused sidecar DESTROYS the panel: `SidecarCmd::Reset` reaches
/// the actor (in-flight turn aborted, conversation dropped) and every
/// sidecar block is purged from the transcript.
#[test]
fn esc_destroys_the_sidecar_panel() {
    let (sidecar_tx, mut sidecar_rx) = mpsc::channel::<crate::sidecar_ui::SidecarCmd>(8);
    let mut chat = ChatView {
        sidecar_focus: true,
        ..ChatView::default()
    };
    chat.blocks.push(ChatBlock::Sidecar {
        id: "sc-1".into(),
        question: "q".into(),
        view: ChatView::default(),
        done: false,
        ok: false,
        answer: None,
        total_tokens: 0,
        rounds: 0,
        started_at_ms: 0,
        elapsed_ms: 0,
    });

    let mut subagent_focus: Option<usize> = None;
    let mut follow = false;
    let mut last_esc = None;
    let mut input = "草稿".to_string();
    let mut cursor = 2usize;
    let mut needs_clear = false;
    let consumed = pre_key_intercept(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &KeyBindings::default(),
        &mut subagent_focus,
        &mut follow,
        &mut last_esc,
        &mut chat,
        &mut input,
        &mut cursor,
        &mut needs_clear,
        &sidecar_tx,
    );

    assert!(consumed);
    assert!(matches!(
        sidecar_rx.try_recv(),
        Ok(crate::sidecar_ui::SidecarCmd::Reset)
    ));
    assert!(
        !chat
            .blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Sidecar { .. })),
        "ESC must purge every sidecar block"
    );
    assert!(!chat.sidecar_focus, "focus released");
    assert!(follow);
    assert_eq!(input, "草稿".to_string(), "draft untouched");
}

/// Ctrl+L on a focused sidecar exits + destroys it, THEN still runs the
/// parent-wide collapse (thinking/tool blocks collapse too).
#[test]
fn ctrl_l_destroys_the_sidecar_then_collapses_parent() {
    let (sidecar_tx, mut sidecar_rx) = mpsc::channel::<crate::sidecar_ui::SidecarCmd>(8);
    let mut chat = ChatView {
        sidecar_focus: true,
        ..ChatView::default()
    };
    chat.blocks.push(ChatBlock::Sidecar {
        id: "sc-1".into(),
        question: "q".into(),
        view: ChatView::default(),
        done: false,
        ok: false,
        answer: None,
        total_tokens: 0,
        rounds: 0,
        started_at_ms: 0,
        elapsed_ms: 0,
    });
    chat.blocks.push(ChatBlock::Thinking {
        text: "思考中...".into(),
        collapsed: false,
        sealed: false,
    });

    let mut subagent_focus: Option<usize> = None;
    let mut follow = false;
    let mut last_esc = None;
    let mut input = "hello".to_string();
    let mut cursor = 5usize;
    let mut needs_clear = false;
    let consumed = pre_key_intercept(
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
        &KeyBindings::default(),
        &mut subagent_focus,
        &mut follow,
        &mut last_esc,
        &mut chat,
        &mut input,
        &mut cursor,
        &mut needs_clear,
        &sidecar_tx,
    );

    assert!(consumed);
    assert!(matches!(
        sidecar_rx.try_recv(),
        Ok(crate::sidecar_ui::SidecarCmd::Reset)
    ));
    assert!(
        !chat
            .blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Sidecar { .. })),
        "Ctrl+L must purge every sidecar block"
    );
    assert!(!chat.sidecar_focus);
    assert!(
        matches!(
            chat.blocks.first(),
            Some(ChatBlock::Thinking {
                collapsed: true,
                ..
            })
        ),
        "parent collapse still ran"
    );
}
