//! Markdown → ratatui `Line` renderer.
//!
//! Uses `pulldown-cmark` to parse CommonMark, then maps the event stream to
//! styled `Line<'static>` / `Span<'static>`. Rendering is deferred to turn
//! completion (never during streaming) so the hot path stays cheap.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::theme;

// ── Exact render shapes (single source of truth) ──────────────────────────
// The decoration glyphs below are spelled out ONLY here. The renderer builds
// its lines from these constants, and copy-mode's structured cleaner
// (`crate::copy_mode::clean`) matches lines against the very same constants —
// if a shape ever drifts, the cleaner follows at compile time instead of
// silently mis-classifying rows at runtime.

/// Leading part of a fenced-code top frame; `flush_code` appends `{label} `
/// (an empty label yields `"┌  "`).
pub(crate) const CODE_TOP_PREFIX: &str = "\u{250c} ";
/// Full bottom frame of a fenced-code block: `└` + 19 × `─`.
pub(crate) const CODE_BOTTOM: &str =
    "\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";
/// A thematic-break (`---`) row: exactly 19 × `─`.
pub(crate) const RULE_LINE: &str =
    "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";
/// Prefix span of every non-empty fenced-code row.
pub(crate) const CODE_ROW_PREFIX: &str = "\u{2502} ";
/// The entire span of an empty fenced-code row.
pub(crate) const CODE_ROW_EMPTY: &str = "\u{2502}";
/// Blockquote prefix. Unlike the code shapes this is `push_str`-ed into the
/// running text span, so it appears at the start of a content span rather
/// than as its own span.
pub(crate) const QUOTE_PREFIX: &str = "\u{258e} ";

/// Render a markdown string into styled ratatui lines.
pub fn render(text: &str) -> Vec<Line<'static>> {
    let text = crate::terminal_text::sanitize_multiline(text);
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(&text, opts);
    let mut r = MdRenderer::new();
    r.process(parser);
    r.finish()
}

struct MdRenderer {
    lines: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    in_code: bool,
    code_lang: String,
    code_buf: Vec<String>,
    list_stack: Vec<(ListKind, usize)>,
    in_para: bool,
    // ── GFM 表格缓冲 ──
    // 表格激活期间 `flush()` 不再落行，而是把取走的 span 收进当前行的
    // 单元格；行末单元格定格为一行，表末统一渲染成对齐网格。
    table_active: bool,
    table_row: Vec<Vec<Span<'static>>>,
    table_rows: Vec<Vec<Vec<Span<'static>>>>,
}

#[derive(Clone, Copy)]
enum ListKind {
    Unordered,
    Ordered,
}

impl MdRenderer {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            spans: Vec::new(),
            style_stack: Vec::new(),
            in_code: false,
            code_lang: String::new(),
            code_buf: Vec::new(),
            list_stack: Vec::new(),
            in_para: false,
            table_active: false,
            table_row: Vec::new(),
            table_rows: Vec::new(),
        }
    }

    fn style(&self) -> Style {
        self.style_stack
            .iter()
            .fold(Style::default(), |a, &b| a.patch(b))
    }

    fn push_str(&mut self, s: String) {
        self.spans.push(Span::styled(s, self.style()));
    }

    fn flush(&mut self) {
        if self.table_active {
            // 表格内：取走的 span 成为当前行的一个单元格。空单元格也必须
            // 占位，否则残行会让后续列整体左移错位。
            self.table_row.push(std::mem::take(&mut self.spans));
        } else if !self.spans.is_empty() || self.in_para {
            self.lines.push(Line::from(std::mem::take(&mut self.spans)));
        }
    }

    fn process<'a, I: Iterator<Item = Event<'a>>>(&mut self, p: I) {
        for ev in p {
            match ev {
                Event::Text(t) => {
                    if self.in_code {
                        self.code_buf.push(t.into_string());
                    } else {
                        self.push_str(t.into_string());
                    }
                }
                Event::Code(c) => {
                    self.spans.push(Span::styled(
                        format!("`{c}`"),
                        self.style().fg(theme::accent()),
                    ));
                }
                Event::SoftBreak | Event::HardBreak => self.flush(),
                Event::Rule => {
                    self.flush();
                    self.lines.push(Line::from(Span::styled(
                        RULE_LINE,
                        Style::default().fg(theme::muted()),
                    )));
                }
                Event::Start(tag) => self.start_tag(tag),
                Event::End(tag) => self.end_tag(tag),
                _ => {}
            }
        }
    }

    fn start_tag(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => self.in_para = true,
            Tag::Heading { level, .. } => {
                let s = match level {
                    pulldown_cmark::HeadingLevel::H1 => Style::default()
                        .fg(theme::warn_color())
                        .add_modifier(Modifier::BOLD),
                    pulldown_cmark::HeadingLevel::H2 => Style::default()
                        .fg(theme::accent())
                        .add_modifier(Modifier::BOLD),
                    _ => Style::default()
                        .fg(theme::info_color())
                        .add_modifier(Modifier::BOLD),
                };
                self.style_stack.push(s);
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                self.in_code = true;
                self.code_lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(l) => l.into_string(),
                    pulldown_cmark::CodeBlockKind::Indented => String::new(),
                };
                self.code_buf.clear();
            }
            Tag::Emphasis => self
                .style_stack
                .push(Style::default().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self
                .style_stack
                .push(Style::default().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self
                .style_stack
                .push(Style::default().add_modifier(Modifier::CROSSED_OUT)),
            Tag::BlockQuote(_) => {
                self.style_stack.push(Style::default().fg(theme::muted()));
                self.push_str(QUOTE_PREFIX.to_string());
            }
            Tag::List(None) => self.list_stack.push((ListKind::Unordered, 0)),
            Tag::List(Some(_)) => self.list_stack.push((ListKind::Ordered, 0)),
            Tag::Item => {
                self.in_para = true;
                let (kind, count) = match self.list_stack.last_mut() {
                    Some(e) => {
                        e.1 += 1;
                        (e.0, e.1)
                    }
                    None => return,
                };
                let indent = "  ".repeat(self.list_stack.len().saturating_sub(1));
                let prefix = match kind {
                    ListKind::Unordered => format!("{indent}\u{2022} "),
                    ListKind::Ordered => format!("{indent}{}. ", count),
                };
                self.push_str(prefix);
            }
            Tag::Link { .. } => {
                self.style_stack.push(
                    self.style()
                        .fg(theme::info_color())
                        .add_modifier(Modifier::UNDERLINED),
                );
                self.push_str("[".to_string());
            }
            Tag::Table(_) => {
                // 先收束挂起的段落，再切换进表格缓冲模式；单元格文本由
                // flush() 落进 table_row，表末统一成型。
                self.flush();
                self.in_para = false;
                self.table_active = true;
                self.table_row.clear();
                self.table_rows.clear();
            }
            // 行是隐式的：单元格直接累积进 table_row，行末定格，无需处理。
            Tag::TableHead | Tag::TableRow => {}
            _ => {
                self.in_para = true;
            }
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush();
                self.in_para = false;
                self.lines.push(Line::from(""));
            }
            TagEnd::Heading(_) => {
                self.flush();
                self.style_stack.pop();
                self.lines.push(Line::from(""));
            }
            TagEnd::CodeBlock => {
                self.flush_code();
                self.in_code = false;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.style_stack.pop();
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.style_stack.pop();
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
            }
            TagEnd::Item => {
                self.flush();
                self.in_para = false;
            }
            TagEnd::Link => {
                self.push_str("]".to_string());
                self.style_stack.pop();
            }
            TagEnd::TableCell => {
                // 单元格闭合：把已累积的 span 落成当前行的一个格子。
                self.flush();
            }
            TagEnd::TableRow | TagEnd::TableHead => {
                self.table_rows.push(std::mem::take(&mut self.table_row));
            }
            TagEnd::Table => {
                let grid = emit_table(&self.table_rows);
                self.lines.extend(grid);
                // 与段落收尾保持一致：表格后留一个空行。
                self.lines.push(Line::from(""));
                self.table_active = false;
                self.table_rows.clear();
                self.in_para = false;
            }
            _ => {}
        }
    }

    fn flush_code(&mut self) {
        let label = if self.code_lang.is_empty() {
            String::new()
        } else {
            self.code_lang.clone()
        };
        self.lines.push(Line::from(Span::styled(
            format!("{CODE_TOP_PREFIX}{label} "),
            Style::default().fg(theme::muted()),
        )));
        // A fenced code block can arrive as a single `Event::Text` whose
        // string contains embedded newlines (pulldown-cmark returns the whole
        // body at once). Flatten every buffered chunk, split on `\n`, and emit
        // one bordered `Line` per logical line — otherwise a multi-line block
        // collapses into one `Line` carrying literal `\n`, which breaks the
        // border and the paragraph line count (scrolling / hit areas).
        let joined: String = self.code_buf.concat();
        let mut rows: Vec<&str> = joined.split('\n').collect();
        // A trailing newline yields a final empty split element that does not
        // correspond to a real line — drop only that single trailing empty, so
        // genuine interior blank lines are preserved.
        if rows.last().is_some_and(|s| s.is_empty()) {
            rows.pop();
        }
        for row in rows {
            // tolerate CRLF endings left in the buffer
            let t = row.strip_suffix('\r').unwrap_or(row);
            if t.is_empty() {
                self.lines.push(Line::from(Span::styled(
                    CODE_ROW_EMPTY,
                    Style::default().fg(theme::muted()),
                )));
            } else {
                self.lines.push(Line::from(vec![
                    Span::styled(CODE_ROW_PREFIX, Style::default().fg(theme::muted())),
                    Span::raw(t.to_string()),
                ]));
            }
        }
        self.lines.push(Line::from(Span::styled(
            CODE_BOTTOM,
            Style::default().fg(theme::muted()),
        )));
        self.lines.push(Line::from(""));
        self.code_buf.clear();
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush();
        while self
            .lines
            .last()
            .map(|l| l.spans.is_empty())
            .unwrap_or(false)
        {
            self.lines.pop();
        }
        self.lines
    }
}

// ── GFM 表格渲染（纯函数：只读缓冲行，产出对齐网格） ──────────────────────

/// 单元格的终端显示宽度：各 span 内容宽度之和（按列计，不按字符数）。
fn table_cell_width(cell: &[Span<'static>]) -> usize {
    cell.iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}

/// 组装一行：单元格之间插入 muted 的 ` │ ` 分隔 span，短单元格右侧补
/// 空格对齐到列宽。`header` 时给每个 span 叠加加粗（patch，保留单元格
/// 自身的内联样式）。残行的缺失单元格渲染为纯空白占位。
fn table_row_line(cells: &[Vec<Span<'static>>], widths: &[usize], header: bool) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (c, width) in widths.iter().enumerate() {
        if c > 0 {
            spans.push(Span::styled(
                " \u{2502} ".to_string(),
                Style::default().fg(theme::muted()),
            ));
        }
        let cell = cells.get(c);
        let used = cell.map(|c| table_cell_width(c)).unwrap_or(0);
        if let Some(cell) = cell {
            for s in cell {
                let mut s = s.clone();
                if header {
                    s.style = s.style.patch(Style::default().add_modifier(Modifier::BOLD));
                }
                spans.push(s);
            }
        }
        let pad = width.saturating_sub(used);
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad)));
        }
    }
    Line::from(spans)
}

/// 把缓冲的表格行渲染成对齐网格：首行为表头（加粗），其后一条
/// `─┼─` 分隔线（每列 `─`×(列宽+2)，用 `┼` 相连，muted 样式），
/// 其余为普通行。列数取最长行；每列宽度取该列单元格的最大显示宽。
/// 残行 / 空单元格不 panic；`rows` 为空或没有列时返回空。超宽表格
/// 不截断、不换行（宽度交给视口处理）。
fn emit_table(rows: &[Vec<Vec<Span<'static>>>]) -> Vec<Line<'static>> {
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if rows.is_empty() || cols == 0 {
        return Vec::new();
    }
    let widths: Vec<usize> = (0..cols)
        .map(|c| {
            rows.iter()
                .map(|r| r.get(c).map(|c| table_cell_width(c)).unwrap_or(0))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut lines = Vec::new();
    let mut first = true;
    // 零单元格的畸形行直接跳过，不产生空行。
    for row in rows.iter().filter(|r| !r.is_empty()) {
        lines.push(table_row_line(row, &widths, first));
        if first {
            first = false;
            lines.push(Line::from(Span::styled(
                widths
                    .iter()
                    .map(|w| "\u{2500}".repeat(w + 2))
                    .collect::<Vec<_>>()
                    .join("\u{253c}"),
                Style::default().fg(theme::muted()),
            )));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading() {
        let ls = render("# Hello");
        assert!(ls
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains("Hello"))));
    }

    #[test]
    fn code_block() {
        let ls = render("```rust\nfn main() {}\n```");
        let t: String = ls
            .iter()
            .flat_map(|l| &l.spans)
            .map(|s| s.content.clone())
            .collect();
        assert!(t.contains("fn main()"), "{t}");
        assert!(t.contains("rust"), "{t}");
    }

    #[test]
    fn multi_line_code_block() {
        // pulldown-cmark returns the whole fenced body as one Text event with
        // embedded newlines; each logical line must become its own bordered Line.
        let ls = render("```rust\nfn a() {}\nfn b() {}\n```");
        let body: Vec<&Line> = ls
            .iter()
            .filter(|l| l.spans.iter().any(|s| s.content.starts_with('\u{2502}')))
            .collect();
        assert_eq!(body.len(), 2, "expected 2 code lines, got {ls:?}");
        for l in &ls {
            for s in &l.spans {
                assert!(
                    !s.content.contains('\n'),
                    "literal newline in span: {:?}",
                    s.content
                );
            }
        }
    }

    #[test]
    fn code_block_with_blank_line() {
        // an interior blank line must still produce its own bordered row.
        let ls = render("```rust\nfn a()\n\nfn b()\n```");
        let body: Vec<&Line> = ls
            .iter()
            .filter(|l| l.spans.iter().any(|s| s.content.starts_with('\u{2502}')))
            .collect();
        assert_eq!(
            body.len(),
            3,
            "expected 3 code lines (incl. blank), got {ls:?}"
        );
    }

    #[test]
    fn crlf_code_block() {
        // CRLF line endings must not leave stray `\r` (or `\n`) in any span.
        let ls = render("```rust\r\nfn a()\r\nfn b()\r\n```");
        for l in &ls {
            for s in &l.spans {
                assert!(
                    !s.content.contains('\r'),
                    "stray CR in span: {:?}",
                    s.content
                );
                assert!(
                    !s.content.contains('\n'),
                    "stray LF in span: {:?}",
                    s.content
                );
            }
        }
        let body: Vec<&Line> = ls
            .iter()
            .filter(|l| l.spans.iter().any(|s| s.content.starts_with('\u{2502}')))
            .collect();
        assert_eq!(body.len(), 2, "expected 2 code lines, got {ls:?}");
    }

    #[test]
    fn bold_italic() {
        let ls = render("**b** *i*");
        assert!(ls
            .iter()
            .flat_map(|l| &l.spans)
            .any(|s| s.style.add_modifier == Modifier::BOLD));
    }

    #[test]
    fn list() {
        let ls = render("- one\n- two");
        let t: String = ls
            .iter()
            .flat_map(|l| &l.spans)
            .map(|s| s.content.clone())
            .collect();
        assert!(t.contains("\u{2022}"), "{t}");
    }

    #[test]
    fn inline_code() {
        let ls = render("use `cargo`");
        assert!(ls
            .iter()
            .flat_map(|l| &l.spans)
            .any(|s| s.content.contains("cargo")));
    }

    #[test]
    fn empty() {
        assert!(render("").is_empty());
    }
}
