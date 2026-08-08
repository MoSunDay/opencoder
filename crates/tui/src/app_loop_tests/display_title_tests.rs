//! Display-title rendering tests extracted from the unified app_loop_tests
//! module to keep each file under the 800-line cap.

use super::*;

// ----- Regression: top title shows workdir · [mode] · bare model id -----

/// Render a styled title `Line` to its plain text (span contents concatenated)
/// for textual assertions.
fn line_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// The top-level `display_title` must strip the `provider/` prefix so the
/// user sees `glm-5.2` rather than the full `bigmodel/glm-5.2`, and follow
/// the order workdir → [mode] → model. Guards against the raw `config.model`
/// leaking through.
#[test]
fn compute_display_title_strips_provider_prefix() {
    use opencoder_core::Config;

    let chat = ChatView {
        agent: "act".to_string(),
        ..ChatView::default()
    };
    let config = Config {
        model: "bigmodel/glm-5.2".to_string(),
        ..Config::default()
    };

    let ds = compute_display(&chat, None, 0, 0, &config, Path::new("/root/opencoder"));

    assert_eq!(
        line_text(&ds.display_title),
        "/root/opencoder \u{00b7} [act] \u{00b7} glm-5.2",
        "top title must be workdir · [mode] · bare model id; got: {}",
        line_text(&ds.display_title)
    );
    assert!(
        !line_text(&ds.display_title).contains("bigmodel"),
        "title must not contain the provider prefix 'bigmodel/': got {}",
        line_text(&ds.display_title)
    );
}

/// With a reasoning-effort badge the prefix must still be stripped and the
/// badge appended last, yielding e.g. "/root/opencoder · [plan] · glm-5.2
/// ·high".
#[test]
fn compute_display_title_with_effort_strips_prefix() {
    use opencoder_core::Config;

    let chat = ChatView {
        agent: "plan".to_string(),
        ..ChatView::default()
    };
    let config = Config {
        model: "bigmodel/glm-5.2".to_string(),
        reasoning_effort: Some("high".to_string()),
        ..Config::default()
    };

    let ds = compute_display(&chat, None, 0, 0, &config, Path::new("/root/opencoder"));

    assert_eq!(
        line_text(&ds.display_title),
        "/root/opencoder \u{00b7} [plan] \u{00b7} glm-5.2 \u{00b7}high",
        "top title order must be workdir → mode → model → effort; got: {}",
        line_text(&ds.display_title)
    );
    assert!(
        !line_text(&ds.display_title).contains("bigmodel"),
        "title must not contain the provider prefix 'bigmodel/': got {}",
        line_text(&ds.display_title)
    );
}

/// A blank `reasoning_effort` must be omitted from the title (same rule as
/// the former status-model badge).
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

    let ds = compute_display(&chat, None, 0, 0, &config, Path::new("/root/opencoder"));

    assert_eq!(
        line_text(&ds.display_title),
        "/root/opencoder \u{00b7} [act] \u{00b7} glm-5.2",
        "blank reasoning_effort must be omitted; got: {}",
        line_text(&ds.display_title)
    );
}
