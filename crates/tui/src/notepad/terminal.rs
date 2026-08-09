//! Terminal log state and command execution for the notepad pseudo-terminal.
//!
//! Commands are run via `sh -c` in the workdir with a 10 s timeout. stdout and
//! stderr are merged. This is a one-shot executor — no interactive PTY support.

use std::path::Path;
use std::time::Duration;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::theme;

const MAX_LINES: usize = 500;
const TIMEOUT_SECS: u64 = 10;

/// One line in the terminal log.
#[derive(Clone, Debug)]
pub struct TermLine {
    pub text: String,
    pub is_command: bool,
}

/// Terminal state: scrolling log + scroll offset.
#[derive(Clone, Debug)]
pub struct TerminalState {
    pub lines: Vec<TermLine>,
    pub scroll: usize,
}

impl TerminalState {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            scroll: 0,
        }
    }

    pub fn push_command(&mut self, cmd: &str) {
        self.lines.push(TermLine {
            text: format!("\u{276f} {}", cmd),
            is_command: true,
        });
        self.trim();
        self.scroll = 0;
    }

    pub fn push_output(&mut self, out: &str) {
        for line in out.lines() {
            self.lines.push(TermLine {
                text: line.to_string(),
                is_command: false,
            });
        }
        // If output didn't end with newline, there's nothing more to do.
        self.trim();
        self.scroll = 0;
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    fn trim(&mut self) {
        if self.lines.len() > MAX_LINES {
            let drop_n = self.lines.len() - MAX_LINES;
            self.lines.drain(0..drop_n);
        }
    }
}

impl Default for TerminalState {
    fn default() -> Self {
        Self::new()
    }
}

/// Run `cmd` via `sh -c` in `workdir`, returning combined stdout + stderr.
/// Times out after [`TIMEOUT_SECS`] seconds.
pub async fn run_command(cmd: &str, workdir: &Path) -> String {
    if cmd.trim().is_empty() {
        return String::new();
    }
    let fut = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(workdir)
        .output();
    match tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), fut).await {
        Ok(Ok(out)) => {
            let mut combined = String::new();
            if !out.stdout.is_empty() {
                combined.push_str(&String::from_utf8_lossy(&out.stdout));
            }
            if !out.stderr.is_empty() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            if combined.is_empty() {
                "(no output)".to_string()
            } else {
                combined
            }
        }
        Ok(Err(e)) => format!("error: {}", e),
        Err(_) => format!("timeout ({}s)", TIMEOUT_SECS),
    }
}

/// Render the terminal panel: log lines on top, composer input at the bottom.
pub fn render_terminal(
    f: &mut Frame,
    area: Rect,
    state: &TerminalState,
    input: &str,
    focused: bool,
) {
    let title = " Terminal ";
    let block = if focused {
        theme::rounded_block_focus(title)
    } else {
        theme::rounded_block(title)
    };
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Reserve 2 lines at the bottom for a blank separator + composer input.
    let composer_h = 2usize;
    let log_h = (inner.height as usize).saturating_sub(composer_h);

    let total = state.lines.len();
    let scroll = state.scroll.min(total);
    let avail = log_h.min(total);
    let start = total.saturating_sub(avail + scroll);
    let end = total.saturating_sub(scroll).max(start);

    let mut lines: Vec<Line> = state.lines[start..end]
        .iter()
        .map(|tl| {
            if tl.is_command {
                Line::from(ratatui::text::Span::styled(
                    tl.text.clone(),
                    Style::default()
                        .fg(theme::local_color())
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::raw(tl.text.clone())
            }
        })
        .collect();

    // Pad to fill the log area so the composer sits at the bottom.
    while lines.len() < log_h {
        lines.push(Line::raw(""));
    }

    // Composer input line.
    let prompt = format!("\u{276f} {}", input);
    lines.push(Line::from(ratatui::text::Span::styled(
        prompt,
        Style::default().fg(theme::accent()),
    )));

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_command_echo() {
        let tmp = tempfile::tempdir().unwrap();
        let out = run_command("echo hello", tmp.path()).await;
        assert!(out.contains("hello"));
    }

    #[tokio::test]
    async fn run_command_stderr_merged() {
        let tmp = tempfile::tempdir().unwrap();
        let out = run_command("echo err >&2", tmp.path()).await;
        assert!(out.contains("err"));
    }

    #[tokio::test]
    async fn run_command_stdout_stderr_both() {
        let tmp = tempfile::tempdir().unwrap();
        let out = run_command("echo out; echo err >&2", tmp.path()).await;
        assert!(out.contains("out"));
        assert!(out.contains("err"));
    }

    #[tokio::test]
    async fn run_command_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let out = run_command("sleep 30", tmp.path()).await;
        assert!(out.contains("timeout"));
    }

    #[tokio::test]
    async fn run_command_empty_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let out = run_command("   ", tmp.path()).await;
        assert_eq!(out, "");
    }

    #[tokio::test]
    async fn run_command_no_output() {
        let tmp = tempfile::tempdir().unwrap();
        let out = run_command("true", tmp.path()).await;
        assert_eq!(out, "(no output)");
    }

    #[tokio::test]
    async fn run_command_independent_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let out = run_command("cd /tmp && pwd", tmp.path()).await;
        assert!(out.trim().ends_with("/tmp"));
    }

    #[test]
    fn push_and_trim() {
        let mut s = TerminalState::new();
        for i in 0..600 {
            s.push_output(&format!("line {}", i));
        }
        assert_eq!(s.lines.len(), MAX_LINES);
    }

    #[test]
    fn scroll_bounds() {
        let mut s = TerminalState::new();
        s.push_output("a\nb\nc");
        s.scroll_up();
        s.scroll_up();
        assert_eq!(s.scroll, 2);
        s.scroll_down();
        assert_eq!(s.scroll, 1);
        s.scroll_down();
        s.scroll_down();
        assert_eq!(s.scroll, 0);
    }
}
