//! Echo-log types for the notepad console.
//!
//! The echo log is a read-only scrolling buffer that records user prompts,
//! bash command/output pairs, and status lines (agent activity, etc.).

const MAX_LINES: usize = 500;

/// Kind tag for styling in the render layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleLineKind {
    /// User-submitted prompt text (`❯ text`).
    User,
    /// Bash command invocation (`❯ !cmd`).
    BashCmd,
    /// Output lines from a bash command.
    BashOut,
    /// Status / informational line (`◆ text`).
    Status,
}

/// One line in the echo log.
#[derive(Clone, Debug)]
pub struct ConsoleLine {
    pub kind: ConsoleLineKind,
    pub text: String,
}

/// Scrollable echo-log buffer.
#[derive(Clone, Debug, Default)]
pub struct EchoLog {
    pub lines: Vec<ConsoleLine>,
    pub scroll: usize,
}

impl EchoLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a user-prompt line.
    pub fn push_user(&mut self, text: &str) {
        self.lines.push(ConsoleLine {
            kind: ConsoleLineKind::User,
            text: format!("\u{276f} {}", text),
        });
        self.reset_scroll();
    }

    /// Push a bash-command echo line.
    pub fn push_bash_cmd(&mut self, cmd: &str) {
        self.lines.push(ConsoleLine {
            kind: ConsoleLineKind::BashCmd,
            text: format!("\u{276f} !{}", cmd),
        });
        self.reset_scroll();
    }

    /// Push bash output (may be multi-line).
    pub fn push_bash_out(&mut self, out: &str) {
        if out.is_empty() {
            self.lines.push(ConsoleLine {
                kind: ConsoleLineKind::BashOut,
                text: "(no output)".to_string(),
            });
        } else {
            for line in out.lines() {
                self.lines.push(ConsoleLine {
                    kind: ConsoleLineKind::BashOut,
                    text: line.to_string(),
                });
            }
        }
        self.reset_scroll();
    }

    /// Push a status line.
    pub fn push_status(&mut self, text: &str) {
        self.lines.push(ConsoleLine {
            kind: ConsoleLineKind::Status,
            text: format!("\u{25c6} {}", text),
        });
        self.reset_scroll();
    }

    /// Clear all lines.
    pub fn clear(&mut self) {
        self.lines.clear();
        self.scroll = 0;
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    fn reset_scroll(&mut self) {
        self.trim();
        self.scroll = 0;
    }

    fn trim(&mut self) {
        if self.lines.len() > MAX_LINES {
            let drop_n = self.lines.len() - MAX_LINES;
            self.lines.drain(0..drop_n);
        }
    }

    /// Number of stored lines (after trimming).
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Whether the log is empty.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_user_adds_line() {
        let mut log = EchoLog::new();
        log.push_user("hello");
        assert_eq!(log.len(), 1);
        assert_eq!(log.lines[0].kind, ConsoleLineKind::User);
        assert!(log.lines[0].text.contains("hello"));
    }

    #[test]
    fn push_bash_out_multiline() {
        let mut log = EchoLog::new();
        log.push_bash_out("a\nb\nc");
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn push_bash_out_empty_shows_placeholder() {
        let mut log = EchoLog::new();
        log.push_bash_out("");
        assert_eq!(log.len(), 1);
        assert_eq!(log.lines[0].text, "(no output)");
    }

    #[test]
    fn clear_empties_log() {
        let mut log = EchoLog::new();
        log.push_user("x");
        log.clear();
        assert!(log.is_empty());
    }

    #[test]
    fn trim_caps_at_max() {
        let mut log = EchoLog::new();
        for i in 0..600 {
            log.push_bash_out(&format!("line {}", i));
        }
        assert_eq!(log.len(), MAX_LINES);
    }

    #[test]
    fn scroll_up_down_bounds() {
        let mut log = EchoLog::new();
        log.push_user("x");
        log.scroll_up();
        log.scroll_up();
        assert_eq!(log.scroll, 2);
        log.scroll_down();
        assert_eq!(log.scroll, 1);
        log.scroll_down();
        log.scroll_down();
        assert_eq!(log.scroll, 0);
    }
}
