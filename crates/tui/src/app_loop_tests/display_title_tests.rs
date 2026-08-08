//! Display-title rendering tests extracted from the unified app_loop_tests
//! module to keep each file under the 800-line cap.

use super::*;

// ----- Regression: top title values share the workdir style -----

/// Render a styled title `Line` to its plain text (span contents concatenated)
/// for textual assertions.
fn line_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// The provider prefix is omitted and workdir/model/effort all use raw spans.
#[test]
fn compute_display_title_uses_workdir_style_for_model_and_effort() {
    use opencoder_core::Config;

    let chat = ChatView {
        agent: "act".to_string(),
        ..ChatView::default()
    };
    let config = Config {
        model: "bigmodel/glm-5.2".to_string(),
        reasoning_effort: Some("high".to_string()),
        ..Config::default()
    };

    let ds = compute_display(
        &chat,
        None,
        0,
        0,
        &config,
        Path::new("/root/opencoder"),
        80,
        crate::app::app_display::TOP_ARROW_W,
    );

    let t = line_text(&ds.display_title);
    assert_eq!(t, "/root/opencoder \u{00b7} glm-5.2 \u{00b7} high");
    assert!(
        !t.contains("bigmodel"),
        "provider prefix must not appear; got: {t}"
    );
    assert!(
        ds.display_title
            .spans
            .iter()
            .all(|span| span.style == Style::default()),
        "model and thinking effort must use the same raw style as workdir"
    );
}

/// A blank `reasoning_effort` is omitted without leaving a separator.
#[test]
fn compute_display_title_omits_blank_effort() {
    use opencoder_core::Config;

    let chat = ChatView {
        agent: "act".to_string(),
        ..ChatView::default()
    };
    let config = Config {
        model: "bigmodel/glm-5.2".to_string(),
        reasoning_effort: Some("   ".to_string()),
        ..Config::default()
    };

    let ds = compute_display(
        &chat,
        None,
        0,
        0,
        &config,
        Path::new("/root/opencoder"),
        80,
        crate::app::app_display::TOP_ARROW_W,
    );

    let t = line_text(&ds.display_title);
    assert_eq!(t, "/root/opencoder \u{00b7} glm-5.2");
    assert!(
        !t.ends_with("\u{00b7}"),
        "no trailing separator after the workdir; got: {t}"
    );
}

/// Subagent focus still swaps in the back/navigation title, unaffected by the
/// top-level title styling change.
#[test]
fn compute_display_subagent_title_keeps_navigation() {
    use opencoder_core::Config;

    let mut chat = ChatView::default();
    chat.blocks.push(crate::chat::ChatBlock::Subagent {
        id: "child-1".into(),
        child_session_id: "sub-s".into(),
        kind: "explore".into(),
        prompt: "investigate".into(),
        view: ChatView::default(),
        done: true,
        ok: true,
        cancelled: false,
        summary: String::new(),
        started_at_ms: 0,
        elapsed_ms: None,
    });

    let ds = compute_display(
        &chat,
        Some(0),
        0,
        0,
        &Config::default(),
        Path::new("/root/opencoder"),
        80,
        crate::app::app_display::TOP_ARROW_W,
    );

    let t = line_text(&ds.display_title);
    assert!(
        t.contains("[Ctrl+L] back"),
        "subagent view must keep its navigation title; got: {t}"
    );
    assert!(
        t.contains("investigate"),
        "subagent prompt stays in the navigation title; got: {t}"
    );
}
