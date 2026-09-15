//! GFM 表格渲染：Say 正文里的表格不再挤成一行，而是渲染成对齐网格
//! （表头加粗 + `─┼─` 分隔线 + 按显示宽对齐的列）。
use super::*;

/// 一行的拼接文本（跨 span 连接，等价于终端上看到的字符）。
fn line_text(l: &Line) -> String {
    l.spans.iter().map(|s| s.content.clone()).collect()
}

fn render_text(src: &str) -> Vec<String> {
    crate::markdown::render(src).iter().map(line_text).collect()
}

#[test]
fn basic_table_header_bold_and_separator() {
    let src = "| Check | Outcome |\n|---|---|\n| Build | Pass |\n| Tests | Fail |\n";
    let ls = crate::markdown::render(src);
    let text: Vec<String> = ls.iter().map(line_text).collect();

    // 表头行：`Check │ Outcome`（U+2502 分隔），且 "Check" span 加粗。
    assert!(
        text[0].contains("Check \u{2502} Outcome"),
        "header row: {:?}\n{text:?}",
        text[0]
    );
    assert!(
        ls[0]
            .spans
            .iter()
            .any(|s| s.content.contains("Check") && s.style.add_modifier.contains(Modifier::BOLD)),
        "header spans must carry BOLD: {:?}",
        ls[0].spans
    );

    // 第二行是 `─…┼─…` 分隔线。
    assert!(
        text[1].contains('\u{253c}') && text[1].chars().all(|c| c == '\u{2500}' || c == '\u{253c}'),
        "separator row: {:?}",
        text[1]
    );

    // 正文行：单元格文本齐全且按列宽补齐（列宽 5 / 7）。
    assert_eq!(text[2], "Build │ Pass   ", "{text:?}");
    assert_eq!(text[3], "Tests │ Fail   ", "{text:?}");

    // 不再出现挤成一行的连接，也不残留原始 `|---` 分隔行。
    assert!(
        !text.iter().any(|t| t.contains("CheckOutcome")),
        "jammed concatenation survived: {text:?}"
    );
    assert!(
        !text.iter().any(|t| t.contains("|---")),
        "raw delimiter row survived: {text:?}"
    );
}

#[test]
fn single_column_and_empty_cells_align_without_panic() {
    // 单列表：没有列分隔符，仍按列宽（"Only" = 4）对齐补空格。
    let text = render_text("| Only |\n|---|\n| a |\n| bb |");
    assert!(text.contains(&"a   ".to_string()), "{text:?}");
    assert!(text.contains(&"bb  ".to_string()), "{text:?}");
    assert!(
        !text.iter().any(|t| t.contains('\u{2502}')),
        "single column must not emit separators: {text:?}"
    );

    // 空单元格：占位不塌陷（补成 1 个空格），后续列仍对齐。
    let text = render_text("| A | B |\n|---|---|\n|  | x |");
    let row = text
        .iter()
        .find(|t| t.contains('x'))
        .unwrap_or_else(|| panic!("body row missing: {text:?}"));
    assert!(row.starts_with(' '), "empty cell must stay padded: {row:?}");
    assert!(!row.contains('|'), "raw pipe survived: {row:?}");
}

#[test]
fn inline_code_span_survives_inside_cells() {
    let ls = crate::markdown::render("| cmd | desc |\n|---|---|\n| run `cargo` | build |\n");
    let span = ls
        .iter()
        .flat_map(|l| &l.spans)
        .find(|s| s.content.contains("`cargo`"))
        .unwrap_or_else(|| panic!("code span lost: {ls:?}"));
    // 反引号文本存活，且保留代码位的 accent 着色。
    assert!(span.content.contains('`'));
    assert_eq!(span.style.fg, Some(crate::theme::accent()));
}

#[test]
fn chat_level_say_table_renders_after_llm_round_end() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 1000,
    });
    v.apply(&SessionEvent::TextDelta(
        "| Check | Outcome |\n|---|---|\n| Build | Ok |\n| Tests | Ok |\n".into(),
    ));
    v.apply(&SessionEvent::LlmRoundEnd);
    let text: Vec<String> = v.flatten().iter().map(line_text).collect();
    let joined = text.join("\n");

    assert!(
        joined.contains("Check \u{2502} Outcome"),
        "rendered header missing:\n{joined}"
    );
    assert!(
        joined.contains('\u{253c}'),
        "column separator missing:\n{joined}"
    );
    assert!(
        !joined.contains("|---"),
        "raw delimiter row must not survive rendering:\n{joined}"
    );
    assert!(
        !joined.contains("CheckOutcome"),
        "cells must not jam into one run:\n{joined}"
    );
}

#[test]
fn separator_cross_aligns_under_column_pipes() {
    use unicode_width::UnicodeWidthStr as Uws;

    fn assert_aligned(src: &str) {
        let ls = crate::markdown::render(src);
        let text: Vec<String> = ls.iter().map(line_text).collect();
        let header = text[0].as_str();
        let sep = text[1].as_str();
        let display_col = |s: &str, byte_idx: usize| Uws::width(&s[..byte_idx]);
        let pipes: Vec<usize> = header
            .match_indices('\u{2502}')
            .map(|(i, _)| display_col(header, i))
            .collect();
        let crosses: Vec<usize> = sep
            .match_indices('\u{253c}')
            .map(|(i, _)| display_col(sep, i))
            .collect();
        assert!(
            !pipes.is_empty() && pipes.len() == crosses.len(),
            "pipe/cross count mismatch:\n{text:?}"
        );
        assert_eq!(
            pipes, crosses,
            "`\u{253c}` must sit directly under `\u{2502}`:\n{text:?}"
        );
        assert_eq!(
            Uws::width(header),
            Uws::width(sep),
            "separator row width must equal header row width:\n{text:?}"
        );
    }

    // ASCII 表与含 CJK 宽字符的表：`┼` 按显示宽落在 `│` 正下方。
    assert_aligned("| Check | Outcome | Notes |\n|---|---|---|\n| Build | Pass | ok |\n");
    assert_aligned("| 检查 | 结果 |\n|---|---|\n| 构建 | 通过 |\n");
}
