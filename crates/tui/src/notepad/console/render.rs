//! Rendering for the console: echo log (top) + modal composer (bottom).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::composer;
use crate::notepad::console::state::{ConsoleLineKind, EchoLog};
use crate::notepad::console::ConsoleState;
use crate::theme;
use crate::vim::{VimMode, VimState};

/// Render the full console (echo + composer) into `area`.
pub fn render_console(f: &mut Frame, area: Rect, state: &ConsoleState, focused: bool) {
    if area.height < 4 {
        return;
    }
    let composer_h: u16 = 5;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(composer_h.min(area.height)),
        ])
        .split(area);

    render_echo(f, chunks[0], &state.echo, focused);
    render_composer(
        f,
        chunks[1],
        &state.vim,
        focused,
        state.running,
        state.bash_running,
    );
}

// ── Echo log ───────────────────────────────────────────────────────────────

fn render_echo(f: &mut Frame, area: Rect, echo: &EchoLog, _focused: bool) {
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        " Console ",
        Style::default().fg(theme::accent()),
    ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let total = echo.lines.len();
    let visible = inner.height as usize;
    // Determine the window of lines to show (bottom-anchored, honour scroll).
    let end = total.saturating_sub(echo.scroll);
    let start = end.saturating_sub(visible);
    let window: Vec<&crate::notepad::console::state::ConsoleLine> = echo.lines
        [start.min(total)..end.min(total)]
        .iter()
        .collect();

    let lines: Vec<Line> = if window.is_empty() {
        vec![Line::from(Span::styled(
            "(empty — type a prompt and press Enter in Normal mode)",
            Style::default().add_modifier(Modifier::DIM),
        ))]
    } else {
        window
            .iter()
            .map(|cl| Line::from(Span::styled(&cl.text, style_for_kind(cl.kind))))
            .collect()
    };

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn style_for_kind(kind: ConsoleLineKind) -> Style {
    match kind {
        ConsoleLineKind::User | ConsoleLineKind::BashCmd => Style::default()
            .fg(theme::accent())
            .add_modifier(Modifier::BOLD),
        ConsoleLineKind::BashOut => Style::default(),
        ConsoleLineKind::Status => Style::default().fg(theme::warn_color()),
    }
}

// ── Composer ───────────────────────────────────────────────────────────────

fn render_composer(
    f: &mut Frame,
    area: Rect,
    vim: &VimState,
    focused: bool,
    agent_running: bool,
    bash_running: bool,
) {
    let title = mode_title(vim, focused, agent_running, bash_running);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, Style::default().fg(theme::accent())));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Render the composer text (one paragraph, lines split by newline).
    let display = if vim.text.is_empty() {
        vec![Line::from(Span::styled(
            "~",
            Style::default().add_modifier(Modifier::DIM),
        ))]
    } else {
        vim.text
            .lines()
            .map(|l| Line::from(l.to_string()))
            .collect::<Vec<_>>()
    };
    let para = Paragraph::new(display);
    f.render_widget(para, inner);

    // Cursor placement (only in Insert / Normal — not Command / Search).
    if vim.mode == VimMode::Insert || vim.mode == VimMode::Normal {
        let (cx, cy) = composer::cursor_screen_position(
            inner.x,
            inner.y,
            &vim.text,
            vim.cursor,
            inner.width,
            0,
            0,
        );
        let clamped_x = cx.min(inner.right().saturating_sub(1));
        let clamped_y = cy.min(inner.bottom().saturating_sub(1));
        f.set_cursor_position((clamped_x, clamped_y));
    }
}

/// Build the composer block title showing vim mode + status.
fn mode_title(vim: &VimState, focused: bool, agent_running: bool, bash_running: bool) -> String {
    let mode_str: String = match vim.mode {
        VimMode::Insert => "-- INSERT --".into(),
        VimMode::Normal => "-- NORMAL --".into(),
        VimMode::Command => format!(":{}", vim.cmdline),
        VimMode::Search => format!("/{}", vim.search_input),
    };
    let char_count = vim.text.chars().count();
    let line_count = vim.text.lines().count().max(1);
    let focus_tag = if focused { "\u{25b8}" } else { "" };
    let status = if bash_running {
        "\u{25c6} running\u{2026}"
    } else if agent_running {
        "\u{25c6} agent responding\u{2026}"
    } else {
        ""
    };
    format!(
        " {} {} [{}L {}C] {} ",
        mode_str, focus_tag, line_count, char_count, status
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn draw(area: Rect, state: &ConsoleState, focused: bool) {
        let backend = TestBackend::new(area.width.max(1), area.height.max(1));
        let mut frame = ratatui::Terminal::new(backend).unwrap();
        frame
            .draw(|f| render_console(f, area, state, focused))
            .unwrap();
    }

    #[test]
    fn render_no_panic_normal() {
        let mut c = ConsoleState::new();
        c.vim.text = "hello\nworld".into();
        c.vim.mode = VimMode::Normal;
        draw(Rect::new(0, 0, 60, 12), &c, true);
    }

    #[test]
    fn render_no_panic_insert_empty() {
        let c = ConsoleState::new();
        draw(Rect::new(0, 0, 60, 12), &c, true);
    }

    #[test]
    fn render_no_panic_command_mode() {
        let mut c = ConsoleState::new();
        c.vim.mode = VimMode::Command;
        c.vim.cmdline = "send".into();
        draw(Rect::new(0, 0, 60, 12), &c, true);
    }

    #[test]
    fn render_tiny_area_no_panic() {
        let c = ConsoleState::new();
        draw(Rect::new(0, 0, 3, 2), &c, true);
    }

    #[test]
    fn render_with_echo_lines() {
        let mut c = ConsoleState::new();
        c.echo.push_user("test prompt");
        c.echo.push_bash_out("output line 1\noutput line 2");
        c.vim.mode = VimMode::Insert;
        draw(Rect::new(0, 0, 60, 12), &c, true);
    }

    #[test]
    fn render_running_status() {
        let mut c = ConsoleState::new();
        c.running = true;
        draw(Rect::new(0, 0, 60, 12), &c, true);
    }
}
