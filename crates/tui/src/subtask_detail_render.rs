//! Renderer for the fullscreen `/subtasks` detail view. Splits the terminal
//! into a registry list band, the selected subtask's four detail sections
//! (`问题` / `过程` / `结果` / `消耗`) and a one-line key hint. The detail
//! pane is scrollable; its rendered line count is written back into
//! [`SubtaskView::last_detail_lines`] so key handling can bound scrolling.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthChar;

use crate::subtask_detail::{SubtaskSnapshot, SubtaskView};
use crate::theme;

/// One-line key hint below the detail pane.
const FOOTER_HINT: &str = "j/k \u{9009}\u{62e9} \u{00b7} r \u{5237}\u{65b0} \u{00b7} c \u{53d6}\u{6d88} \u{00b7} Enter \u{5bf9}\u{8bdd}(agent/operator) \u{00b7} Ctrl+Enter \u{8ffd}\u{52a0}\u{56de}\u{4e3b}\u{4efb}\u{52a1} \u{00b7} Esc \u{5173}\u{95ed}";
/// Placeholder while the registry is empty.
const EMPTY_HINT: &str = "\u{6ca1}\u{6709}\u{5df2}\u{6ce8}\u{518c}\u{7684} subtask\u{ff08}@<capability> <\u{95ee}\u{9898}> \u{63d0}\u{4ea4}\u{4e00}\u{4e2a}\u{ff09}";
/// Section headers, in render order.
const SECTIONS: [&str; 4] = ["\u{95ee}\u{9898}", "\u{8fc7}\u{7a0b}", "\u{7ed3}\u{679c}", "\u{6d88}\u{8017}"];

/// Render the fullscreen view over `area`.
pub(crate) fn render(f: &mut Frame, area: Rect, view: &SubtaskView) {
    if view.entries.is_empty() {
        let block = theme::rounded_block("subtasks");
        let text = Paragraph::new(vec![
            Line::from(Span::styled(
                EMPTY_HINT,
                Style::default().fg(theme::muted()),
            )),
            Line::raw(""),
            Line::from(Span::styled(FOOTER_HINT, Style::default().fg(theme::muted()))),
        ])
        .block(block)
        .wrap(Wrap { trim: false });
        f.render_widget(text, area);
        view.last_detail_lines.set(0);
        return;
    }
    // List band: one row per entry (plus the block border), capped so the
    // detail pane keeps at least a few rows plus the footer.
    let list_h = (view.entries.len() as u16 + 2).clamp(3, area.height.saturating_sub(4).max(3));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(list_h),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);
    render_list(f, chunks[0], view);
    let inner_w = chunks[1].width.saturating_sub(2) as usize;
    let lines = detail_lines(view, inner_w);
    view.last_detail_lines.set(lines.len() as u32);
    let visible = chunks[1].height.saturating_sub(2) as usize;
    let shown: Vec<Line> = lines
        .into_iter()
        .skip(view.scroll as usize)
        .take(visible.max(1))
        .collect();
    let detail = Paragraph::new(shown)
        .block(theme::rounded_block("subtask detail"))
        .wrap(Wrap { trim: false });
    f.render_widget(detail, chunks[1]);
    if chunks[2].height > 0 {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                FOOTER_HINT,
                Style::default().fg(theme::muted()),
            ))),
            chunks[2],
        );
    }
}

/// The registry list band; the selected row carries the highlight style.
fn render_list(f: &mut Frame, area: Rect, view: &SubtaskView) {
    let items: Vec<ListItem> = view
        .entries
        .iter()
        .map(|entry| {
            ListItem::new(Line::from(vec![
                Span::raw("\u{25aa} "),
                Span::styled(
                    entry.capability_id.clone(),
                    Style::default()
                        .fg(theme::accent())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    " [{}] {} tokens={}",
                    entry.kind, entry.status, entry.tokens
                )),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(theme::rounded_block("subtasks"))
        .highlight_style(theme::list_highlight());
    let mut state = ListState::default();
    state.select(Some(view.selected));
    f.render_stateful_widget(list, area, &mut state);
}

/// The selected subtask's four sections as wrapped, styled lines. The
/// snapshot (when fetched) supplies the timeline and the freshest summary;
/// registry fields cover the rest.
fn detail_lines(view: &SubtaskView, width: usize) -> Vec<Line<'static>> {
    let Some(entry) = view.selected_entry() else {
        return Vec::new();
    };
    let snapshot: Option<&SubtaskSnapshot> = view.snapshots.get(&entry.execution_id);
    let mut lines = Vec::new();
    // 问题
    lines.push(header_line(SECTIONS[0]));
    lines.extend(wrap_lines(&entry.question, width));
    // 过程
    lines.push(header_line(SECTIONS[1]));
    match snapshot.map(|snapshot| snapshot.timeline.as_slice()) {
        Some([]) | None => lines.push(Line::from(Span::styled(
            "\u{ff08}\u{6309} r \u{62c9}\u{53d6}\u{8fc7}\u{7a0b}\u{65f6}\u{95f4}\u{7ebf}\u{ff09}",
            Style::default().fg(theme::muted()),
        ))),
        Some(rows) => lines.extend(rows.iter().cloned().map(Line::raw)),
    }
    // 结果
    lines.push(header_line(SECTIONS[2]));
    let summary = snapshot
        .and_then(|snapshot| snapshot.summary.clone())
        .or_else(|| entry.result_summary.clone());
    match summary {
        Some(summary) => lines.extend(wrap_lines(&summary, width)),
        None => lines.push(Line::from(Span::styled(
            "\u{ff08}\u{672a}\u{5b8c}\u{6210}\u{ff09}",
            Style::default().fg(theme::muted()),
        ))),
    }
    // 消耗
    lines.push(header_line(SECTIONS[3]));
    lines.push(Line::raw(format!(
        "tokens: {} \u{00b7} status: {} \u{00b7} created_at: {}",
        entry.tokens, entry.status, entry.created_at
    )));
    lines
}

/// One section header line (bold accent).
fn header_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default()
            .fg(theme::accent())
            .add_modifier(Modifier::BOLD),
    ))
}

/// Greedy char-boundary wrap by display width (multi-byte safe). Empty text
/// still yields one row so every section keeps at least one content line.
fn wrap_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut rows: Vec<String> = Vec::new();
    let mut row = String::new();
    let mut col = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            rows.push(std::mem::take(&mut row));
            col = 0;
            continue;
        }
        let char_w = ch.width().unwrap_or(0);
        if col + char_w > width && col > 0 {
            rows.push(std::mem::take(&mut row));
            col = 0;
        }
        col += char_w;
        row.push(ch);
    }
    rows.push(row);
    rows.into_iter().map(Line::raw).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::harness::SubtaskRef;
    use ratatui::backend::TestBackend;

    fn entry(id: &str, status: &str) -> SubtaskRef {
        SubtaskRef {
            execution_id: id.into(),
            kind: "agent".into(),
            capability_id: "ops_x".into(),
            question: "check the logs".into(),
            status: status.into(),
            result_summary: None,
            tokens: 7,
            created_at: 1,
        }
    }

    /// Render into a TestBackend terminal; returns (visible text with the
    /// wide-char continuation cells skipped, the recorded detail line count).
    fn draw(view: &SubtaskView) -> (String, u32) {
        let mut terminal = ratatui::Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render(f, f.area(), view))
            .unwrap();
        let content = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .filter(|cell| !cell.symbol().trim().is_empty())
            .map(|cell| cell.symbol().to_string())
            .collect();
        (content, view.last_detail_lines.get())
    }

    #[test]
    fn empty_registry_renders_placeholder_without_panic() {
        let view = SubtaskView::new(Vec::new());
        let (content, lines) = draw(&view);
        let dense_hint: String = EMPTY_HINT.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(content.contains(&dense_hint), "placeholder must render");
        assert_eq!(lines, 0);
    }

    #[test]
    fn detail_sections_render_headers_and_content() {
        let view = SubtaskView::new(vec![entry("agent-a", "done")]);
        let (content, lines) = draw(&view);
        assert!(content.contains("\u{95ee}\u{9898}"), "question header");
        assert!(content.contains("checkthelogs"));
        assert!(content.contains("\u{8fc7}\u{7a0b}"), "timeline header");
        assert!(content.contains("\u{7ed3}\u{679c}"), "result header");
        assert!(content.contains("\u{6d88}\u{8017}"), "cost header");
        assert!(content.contains("tokens:7"));
        assert!(lines > 0, "scroll bound recorded");
    }

    #[test]
    fn wrap_lines_breaks_on_display_width_not_chars() {
        let rows: Vec<String> = wrap_lines("ab cd", 2)
            .into_iter()
            .map(|line| line.spans.iter().map(|s| s.content.to_string()).collect())
            .collect();
        assert_eq!(rows, vec!["ab", " c", "d"]);
        let wide = wrap_lines("\u{6c49}\u{5b57}\u{6c49}", 2);
        assert_eq!(wide.len(), 3, "one CJK char per 2-col row");
        assert_eq!(wrap_lines("", 3).len(), 1);
    }
}

