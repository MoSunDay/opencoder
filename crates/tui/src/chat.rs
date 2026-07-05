use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use opencode_session::SessionEvent;

/// Maximum lines of tool output to render inline.
const TOOL_OUTPUT_LINES: usize = 6;

#[derive(Default)]
pub struct ChatView {
    pub lines: Vec<Line<'static>>,
    pub agent: String,
    pub status: String,
    /// How many lines have been pushed since the UI last auto-scrolled.
    pub dirty: bool,
}

impl ChatView {
    pub fn apply(&mut self, ev: &SessionEvent) {
        match ev {
            SessionEvent::TextDelta(t) => self.push_raw(t),
            SessionEvent::ReasoningDelta(_) => {}
            SessionEvent::ToolStart { name, input, .. } => {
                self.lines.push(Line::from(vec![
                    Span::styled(format!("\u{25b8} {name} "), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(summarize(input), Style::default().fg(Color::DarkGray)),
                ]));
                self.dirty = true;
            }
            SessionEvent::ToolEnd { output, is_error, .. } => {
                let color = if *is_error { Color::Red } else { Color::DarkGray };
                for l in output.lines().take(TOOL_OUTPUT_LINES) {
                    self.lines.push(Line::from(Span::styled(format!("  {l}"), Style::default().fg(color))));
                }
                self.dirty = true;
            }
            SessionEvent::AgentSwitch(to) => {
                self.agent = to.clone();
                self.lines.push(Line::from(Span::styled(
                    format!("[switched to {to} mode]"),
                    Style::default().fg(Color::Magenta),
                )));
                self.dirty = true;
            }
            SessionEvent::Compaction(c) => {
                self.lines.push(Line::from(Span::styled(
                    format!("[context compacted] {}", short(c, 100)),
                    Style::default().fg(Color::Yellow),
                )));
                self.dirty = true;
            }
            SessionEvent::Status(s) => self.status = s.clone(),
            SessionEvent::SubagentStart { kind, prompt, .. } => {
                self.lines.push(Line::from(vec![
                    Span::styled("\u{2937} subagent ", Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)),
                    Span::styled(kind.clone(), Style::default().fg(Color::Blue)),
                    Span::styled(format!("  {}", short(prompt, 90)), Style::default().fg(Color::DarkGray)),
                ]));
                self.dirty = true;
            }
            SessionEvent::SubagentEnd { ok, summary, .. } => {
                let mark = if *ok { "\u{2714}" } else { "\u{2718}" };
                let color = if *ok { Color::Green } else { Color::Red };
                self.lines.push(Line::from(vec![
                    Span::styled(format!("  {mark} subagent "), Style::default().fg(color)),
                    Span::styled(short(summary, 110), Style::default().fg(Color::DarkGray)),
                ]));
                self.dirty = true;
            }
            SessionEvent::Done => {
                self.lines.push(Line::from(""));
                self.dirty = true;
            }
            SessionEvent::Error(e) => {
                self.lines.push(Line::from(Span::styled(
                    format!("error: {e}"),
                    Style::default().fg(Color::Red),
                )));
                self.dirty = true;
            }
        }
    }

    fn push_raw(&mut self, t: &str) {
        let newlines = t.matches('\n').count();
        if let Some(last) = self.lines.last_mut() {
            if !last.spans.is_empty() && newlines == 0 {
                last.spans.push(Span::raw(t.to_string()));
                return;
            }
        }
        for (i, part) in t.split_inclusive('\n').enumerate() {
            if i == 0 {
                if let Some(last) = self.lines.last_mut() {
                    last.spans.push(Span::raw(part.to_string()));
                    continue;
                }
            }
            self.lines.push(Line::from(Span::raw(part.to_string())));
        }
        self.dirty = true;
    }

    /// Render the transcript with vertical scroll. Only the visible window is
    /// laid out by ratatui (O(visible)), keeping long sessions fast.
    pub fn render_paragraph(&self, scroll: u16) -> Paragraph<'_> {
        Paragraph::new(self.lines.clone())
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
    }
}

fn summarize(input: &serde_json::Value) -> String {
    match input {
        serde_json::Value::Object(m) => {
            for k in ["command", "path", "description", "pattern", "prompt"] {
                if let Some(s) = m.get(k).and_then(|v| v.as_str()) {
                    return short(s, 80);
                }
            }
            short(&serde_json::to_string(input).unwrap_or_default(), 80)
        }
        o => short(&serde_json::to_string(o).unwrap_or_default(), 80),
    }
}

fn short(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= n {
        t.to_string()
    } else {
        format!("{}...", t.chars().take(n).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencode_session::SessionEvent;

    #[test]
    fn apply_text_delta_appends_to_lines() {
        let mut view = ChatView::default();
        view.apply(&SessionEvent::TextDelta("hello ".into()));
        view.apply(&SessionEvent::TextDelta("world".into()));
        // After two deltas, at least one line should contain the text
        let all_text: String = view.lines.iter().flat_map(|l| l.spans.iter()).map(|s| s.content.clone()).collect();
        assert!(all_text.contains("hello"));
        assert!(all_text.contains("world"));
    }

    #[test]
    fn apply_subagent_start_adds_subagent_header() {
        let mut view = ChatView::default();
        view.apply(&SessionEvent::SubagentStart {
            id: "sub-1".into(),
            kind: "explore".into(),
            prompt: "research the codebase".into(),
        });
        let all_text: String = view.lines.iter().flat_map(|l| l.spans.iter()).map(|s| s.content.clone()).collect();
        assert!(all_text.contains("subagent"));
        assert!(all_text.contains("explore"));
    }

    #[test]
    fn apply_subagent_end_shows_result() {
        let mut view = ChatView::default();
        view.apply(&SessionEvent::SubagentEnd {
            id: "sub-1".into(),
            ok: true,
            summary: "found 3 files".into(),
        });
        let all_text: String = view.lines.iter().flat_map(|l| l.spans.iter()).map(|s| s.content.clone()).collect();
        assert!(all_text.contains("found 3 files"));
    }

    #[test]
    fn apply_error_adds_error_line() {
        let mut view = ChatView::default();
        view.apply(&SessionEvent::Error("something broke".into()));
        let all_text: String = view.lines.iter().flat_map(|l| l.spans.iter()).map(|s| s.content.clone()).collect();
        assert!(all_text.contains("something broke"));
    }
}
