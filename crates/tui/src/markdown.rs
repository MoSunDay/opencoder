//! Markdown → ratatui `Line` renderer.
//!
//! Uses `pulldown-cmark` to parse CommonMark, then maps the event stream to
//! styled `Line<'static>` / `Span<'static>`. Rendering is deferred to turn
//! completion (never during streaming) so the hot path stays cheap.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Render a markdown string into styled ratatui lines.
pub fn render(text: &str) -> Vec<Line<'static>> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(text, opts);
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
}

#[derive(Clone, Copy)]
enum ListKind { Unordered, Ordered }

impl MdRenderer {
    fn new() -> Self {
        Self { lines: Vec::new(), spans: Vec::new(), style_stack: Vec::new(),
            in_code: false, code_lang: String::new(), code_buf: Vec::new(),
            list_stack: Vec::new(), in_para: false }
    }

    fn style(&self) -> Style {
        self.style_stack.iter().fold(Style::default(), |a, &b| a.patch(b))
    }

    fn push_str(&mut self, s: String) {
        self.spans.push(Span::styled(s, self.style()));
    }

    fn flush(&mut self) {
        if !self.spans.is_empty() || self.in_para {
            self.lines.push(Line::from(std::mem::take(&mut self.spans)));
        }
    }

    fn process<'a, I: Iterator<Item = Event<'a>>>(&mut self, p: I) {
        for ev in p {
            match ev {
                Event::Text(t) => {
                    if self.in_code { self.code_buf.push(t.into_string()); }
                    else { self.push_str(t.into_string()); }
                }
                Event::Code(c) => {
                    self.spans.push(Span::styled(
                        format!("`{c}`"),
                        self.style().fg(Color::Cyan)));
                }
                Event::SoftBreak | Event::HardBreak => self.flush(),
                Event::Rule => {
                    self.flush();
                    self.lines.push(Line::from(Span::styled(
                        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
                        Style::default().fg(Color::DarkGray))));
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
                    pulldown_cmark::HeadingLevel::H1 => Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    pulldown_cmark::HeadingLevel::H2 => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    _ => Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
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
            Tag::Emphasis => self.style_stack.push(Style::default().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.style_stack.push(Style::default().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.style_stack.push(Style::default().add_modifier(Modifier::CROSSED_OUT)),
            Tag::BlockQuote(_) => {
                self.style_stack.push(Style::default().fg(Color::DarkGray));
                self.push_str("\u{258e} ".to_string());
            }
            Tag::List(None) => self.list_stack.push((ListKind::Unordered, 0)),
            Tag::List(Some(_)) => self.list_stack.push((ListKind::Ordered, 0)),
            Tag::Item => {
                self.in_para = true;
                let (kind, count) = match self.list_stack.last_mut() {
                    Some(e) => { e.1 += 1; (e.0, e.1) }
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
                self.style_stack.push(self.style().fg(Color::Blue).add_modifier(Modifier::UNDERLINED));
                self.push_str("[".to_string());
            }
            _ => { self.in_para = true; }
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => { self.flush(); self.in_para = false; self.lines.push(Line::from("")); }
            TagEnd::Heading(_) => { self.flush(); self.style_stack.pop(); self.lines.push(Line::from("")); }
            TagEnd::CodeBlock => { self.flush_code(); self.in_code = false; }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => { self.style_stack.pop(); }
            TagEnd::BlockQuote(_) => { self.flush(); self.style_stack.pop(); }
            TagEnd::List(_) => { self.list_stack.pop(); }
            TagEnd::Item => { self.flush(); self.in_para = false; }
            TagEnd::Link => { self.push_str("]".to_string()); self.style_stack.pop(); }
            _ => {}
        }
    }

    fn flush_code(&mut self) {
        let label = if self.code_lang.is_empty() { String::new() } else { self.code_lang.clone() };
        self.lines.push(Line::from(Span::styled(
            format!("\u{250c} {label} "), Style::default().fg(Color::DarkGray))));
        for line in &self.code_buf {
            let t = line.trim_end_matches('\n');
            if t.is_empty() {
                self.lines.push(Line::from(Span::styled("\u{2502}", Style::default().fg(Color::DarkGray))));
            } else {
                self.lines.push(Line::from(vec![
                    Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
                    Span::raw(t.to_string()),
                ]));
            }
        }
        self.lines.push(Line::from(Span::styled(
            "\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
            Style::default().fg(Color::DarkGray))));
        self.lines.push(Line::from(""));
        self.code_buf.clear();
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush();
        while self.lines.last().map(|l| l.spans.is_empty()).unwrap_or(false) {
            self.lines.pop();
        }
        self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading() {
        let ls = render("# Hello");
        assert!(ls.iter().any(|l| l.spans.iter().any(|s| s.content.contains("Hello"))));
    }

    #[test]
    fn code_block() {
        let ls = render("```rust\nfn main() {}\n```");
        let t: String = ls.iter().flat_map(|l| &l.spans).map(|s| s.content.clone()).collect();
        assert!(t.contains("fn main()"), "{t}");
        assert!(t.contains("rust"), "{t}");
    }

    #[test]
    fn bold_italic() {
        let ls = render("**b** *i*");
        assert!(ls.iter().flat_map(|l| &l.spans).any(|s| s.style.add_modifier == Modifier::BOLD));
    }

    #[test]
    fn list() {
        let ls = render("- one\n- two");
        let t: String = ls.iter().flat_map(|l| &l.spans).map(|s| s.content.clone()).collect();
        assert!(t.contains("\u{2022}"), "{t}");
    }

    #[test]
    fn inline_code() {
        let ls = render("use `cargo`");
        assert!(ls.iter().flat_map(|l| &l.spans).any(|s| s.content.contains("cargo")));
    }

    #[test]
    fn empty() { assert!(render("").is_empty()); }
}
