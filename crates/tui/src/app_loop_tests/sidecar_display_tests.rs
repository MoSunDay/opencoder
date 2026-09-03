//! `compute_display` sidecar-ctx-switch tests: a focused sidecar box swaps
//! the body for the block's nested view, retitles with the back hint, and
//! flips the mode chip to `sidecar` — without touching the main task's
//! running state or the shared system-prompt token count.

use super::*;

use crate::chat::{block_text, SidecarPanel};

fn line_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn sidecar_chat() -> ChatView {
    let mut chat = ChatView {
        agent: "act".to_string(),
        ..ChatView::default()
    };
    // Real entry flow: `/sidecar` pushes the placeholder and opens the panel;
    // only an OPEN panel accepts the conversation's SidecarStart.
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::sidecar_ui::SidecarCmd>(1);
    crate::sidecar_ui::enter_panel(&mut chat, &tx);
    chat.apply(&SessionEvent::SidecarStart {
        id: "sc-1".into(),
        question: "这段代码做什么?".into(),
    });
    chat.apply(&SessionEvent::SidecarChild {
        id: "sc-1".into(),
        ev: Box::new(SessionEvent::TextDelta("旁路答案内容".into())),
    });
    chat
}

#[allow(clippy::too_many_arguments)]
fn display_of(chat: &ChatView) -> DisplayState<'_> {
    compute_display(
        chat,
        None,
        0,
        777,
        &Config::default(),
        Path::new("/root/opencoder"),
        80,
        crate::app::app_display::TOP_ARROW_W,
    )
}

/// Focused sidecar: body = the block's nested view (child content visible),
/// the mode chip reads `sidecar`, and the title carries the Ctrl+L hint
/// plus the echoed question.
#[test]
fn focused_sidecar_swaps_body_mode_and_title() {
    let chat = sidecar_chat();
    let ds = display_of(&chat);
    assert_eq!(ds.display_mode, "sidecar");
    let title = line_text(&ds.display_title);
    assert!(title.contains("Ctrl+L"), "back hint in title, got {title}");
    assert!(title.contains("sidecar"), "kind chip in title");
    assert!(
        title.contains("这段代码做什么?"),
        "question echoed in title"
    );
    assert!(
        block_text(ds.display_chat).contains("旁路答案内容"),
        "body must be the sidecar block's nested view"
    );
    assert_eq!(
        ds.display_ctx,
        chat.sidecar.as_ref().expect("panel").total_tokens,
        "ctx meter reads the sidecar conversation's accumulated tokens"
    );
}

/// Unfocused (destroy exit): the plain parent transcript returns and the
/// sidecar block contributes ZERO lines — the bypass Q/A is not a
/// transcript artifact.
#[test]
fn unfocused_sidecar_restores_the_parent_body() {
    let mut chat = sidecar_chat();
    crate::chat::sidecar::purge(&mut chat);
    let ds = display_of(&chat);
    assert_eq!(ds.display_mode, "act");
    assert!(
        !block_text(ds.display_chat).contains("旁路答案内容"),
        "child content hidden again after exit"
    );
    assert!(
        !block_text(ds.display_chat).contains("sidecar"),
        "the block leaves zero trace in the parent transcript"
    );
}

/// Freshly entered panel (empty placeholder): the body swaps in empty and the
/// title keeps only the nav element — no help hint, the question echoes in the
/// body the moment it is submitted.
#[test]
fn empty_placeholder_panel_title_is_nav_only() {
    let mut chat = ChatView {
        agent: "act".to_string(),
        ..ChatView::default()
    };
    chat.sidecar = Some(SidecarPanel::default());
    chat.sidecar_focus = true;
    let ds = display_of(&chat);
    assert_eq!(ds.display_mode, "sidecar");
    let title = line_text(&ds.display_title);
    assert!(
        title.contains("Ctrl+L") && title.contains("sidecar"),
        "empty panel title keeps the nav element, got {title}"
    );
    assert!(
        !title.contains("输入问题") && !title.contains("就绪") && !title.contains("销毁返回"),
        "no help copy in the empty panel title, got {title}"
    );
}

/// Sidecar focus takes precedence over the ctx meter inputs: it shows the
/// sidecar view's own spend, not the parent's (sys tokens pass through for
/// the shared system prompt).
#[test]
fn focused_sidecar_ctx_is_scoped_to_the_sidecar_view() {
    let mut chat = sidecar_chat();
    chat.apply(&SessionEvent::LlmUsage {
        total_tokens: 5000,
        input_tokens: 4900,
        output_tokens: 100,
    });
    chat.apply(&SessionEvent::SidecarTurn {
        id: "sc-1".into(),
        ok: true,
        answer: "ok".into(),
        elapsed_ms: 1,
        total_tokens: 12,
        rounds: 1,
    });
    let ds = display_of(&chat);
    assert_eq!(ds.display_ctx, 12, "sidecar ctx excludes the parent spend");
    assert_eq!(ds.display_sys, 777, "system-prompt tokens are shared");
}

/// Entering `/sidecar` must not surface the empty-session tutorial: the
/// focused panel's fresh nested view is display-swapped into the body, and
/// the tutorial gate (`body_is_top_level`) must treat it as non-top-level —
/// same rule as a focused subagent child view.
#[test]
fn focused_sidecar_body_is_not_top_level() {
    let mut chat = ChatView {
        agent: "act".to_string(),
        ..ChatView::default()
    };
    assert!(
        super::body_is_top_level(&chat, None),
        "plain main transcript is top level"
    );
    chat.sidecar = Some(SidecarPanel::default());
    chat.sidecar_focus = true;
    assert!(
        !super::body_is_top_level(&chat, None),
        "focused sidecar panel is not top level (no tutorial in the panel body)"
    );
    assert!(
        !super::body_is_top_level(&chat, Some(0)),
        "focused subagent child view is not top level"
    );
}
