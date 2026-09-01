use super::super::*;
use crate::theme;

/// The `User` block renders a gold `❯ User:` header followed by 4-space
/// indented markdown body lines.
#[test]
fn user_block_renders_gold_tag_and_indented_body() {
    let rendered = crate::markdown::render("# hi\n\n- a");
    let view = ChatView {
        blocks: vec![ChatBlock::User { rendered }],
        ..Default::default()
    };
    let flat = view.flatten_with(0, 0);
    let joined: Vec<String> = flat
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();

    // First line is the gold bold header.
    assert!(
        joined[0].contains("User:"),
        "first line must contain 'User:', got {:?}",
        joined[0]
    );
    // Body lines must be indented with 4 spaces.
    for line in &joined[1..] {
        assert!(
            line.starts_with("    "),
            "body line must be 4-space indented, got {:?}",
            line
        );
    }
    // The header span carries the user_color fg.
    let header_span = flat[0]
        .spans
        .iter()
        .find(|s| s.content.contains("User:"))
        .expect("header span exists");
    assert_eq!(header_span.style.fg, Some(theme::user_color()));
}

/// The number of lines emitted by `flatten_with` for a `User` block must
/// match what `collect_headers` accounts (1 header + rendered.len()).
#[test]
fn user_block_line_count_matches_collect_headers() {
    let rendered = crate::markdown::render("line1\n\nline2\n\nline3");
    let n = rendered.len();
    let view = ChatView {
        blocks: vec![
            ChatBlock::User {
                rendered: rendered.clone(),
            },
            ChatBlock::Marker(vec![Line::from("")]),
        ],
        ..Default::default()
    };
    let flat_len = view.flatten_with(0, 0).len();
    // 1 (header) + n (body) + 1 (trailing marker)
    assert_eq!(flat_len, 1 + n + 1);
}

/// `push_user` creates a `ChatBlock::User` with markdown-rendered body.
#[test]
fn push_user_creates_user_block_with_markdown() {
    let mut chat = ChatView::default();
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    crate::app_helpers::push_user(&mut chat, &mut history, &mut hist_idx, "**bold** text", "**bold** text");

    let user_blocks: Vec<_> = chat
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::User { .. }))
        .collect();
    assert_eq!(user_blocks.len(), 1, "exactly one User block expected");
    if let ChatBlock::User { rendered } = user_blocks[0] {
        let text: String = rendered
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect();
        assert!(
            text.contains("bold"),
            "rendered body must contain the markdown text, got: {text}"
        );
    }
}
