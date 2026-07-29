//! Help popup rendering with display-width-aware word wrapping and scroll.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::composer;

/// Word-wrap a single source line into multiple display lines that each fit
/// `max_w` display columns. Uses `composer::char_width` so CJK / wide chars
/// are handled correctly. Breaks at the last space before overflow; if a
/// single word exceeds `max_w` it is hard-broken.
fn wrap_line(text: &str, max_w: usize) -> Vec<String> {
    if max_w == 0 {
        return vec![text.to_string()];
    }
    let mut result: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_w: usize = 0;
    let mut last_space: Option<usize> = None; // byte offset in `current`

    for ch in text.chars() {
        let cw = composer::char_width(ch);
        // Update last_space BEFORE the overflow check so a space at the
        // break boundary is treated as the break point itself.
        if ch == ' ' {
            last_space = Some(current.len());
        }
        if current_w + cw > max_w && !current.is_empty() {
            // Break at last space if possible
            if let Some(sp) = last_space {
                let remainder: String = current[sp..].trim_start().to_string();
                let head = current[..sp].trim_end().to_string();
                result.push(head);
                current = remainder;
                current_w = current.chars().map(composer::char_width).sum();
            } else {
                result.push(std::mem::take(&mut current));
                current_w = 0;
            }
            last_space = None;
            // If the overflowing char is a space, skip it — it was the
            // break point, not content on the new line.
            if ch == ' ' {
                continue;
            }
        }
        current.push(ch);
        current_w += cw;
    }
    if !current.is_empty() {
        result.push(current);
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

/// Build the wrapped help lines from the HELP constant, fitting `max_w`
/// display columns per line.
fn build_wrapped_lines(max_w: usize) -> Vec<String> {
    crate::keybind::HELP
        .lines()
        .flat_map(|line| wrap_line(line, max_w))
        .collect()
}

/// Render the help popup centered on `area`, scrolled by `scroll` lines.
pub fn render_help(f: &mut Frame, area: Rect, scroll: u16) {
    let h = 22u16.min(area.height.saturating_sub(2));
    let w = 62u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let inner_w = (w.saturating_sub(2) as usize).max(1);
    let wrapped = build_wrapped_lines(inner_w);
    let lines: Vec<Line> = wrapped
        .iter()
        .map(|s| Line::from(Span::styled(s.as_str(), Style::default().fg(Color::Gray))))
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" \u{5e2e}\u{52a9} (Ctrl+H \u{6253}\u{5f00}/\u{5173}\u{95ed}, Esc \u{5173}\u{95ed}, \u{2191}\u{2193} \u{6eda}\u{52a8}) ")
        .border_style(Style::default().fg(Color::Cyan));
    f.render_widget(
        Paragraph::new(lines).scroll((scroll, 0)).block(block),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_line_short_passthrough() {
        let result = wrap_line("hello", 80);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn wrap_line_breaks_at_space() {
        let result = wrap_line("aaa bbb ccc", 7);
        assert_eq!(result, vec!["aaa bbb", "ccc"]);
    }

    #[test]
    fn wrap_line_long_word_hard_break() {
        let result = wrap_line("abcdefghij", 4);
        assert!(result.len() > 1);
        for line in &result {
            assert!(line.chars().count() <= 4);
        }
    }

    #[test]
    fn wrap_line_cjk_aware() {
        let result = wrap_line("你好世界", 4);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "你好");
        assert_eq!(result[1], "世界");
    }

    #[test]
    fn wrap_line_empty() {
        let result = wrap_line("", 80);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn build_wrapped_lines_nonempty() {
        let lines = build_wrapped_lines(40);
        assert!(!lines.is_empty());
    }
}
