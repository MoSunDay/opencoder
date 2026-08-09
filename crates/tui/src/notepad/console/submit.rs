//! Submit parsing and background bash execution for the console.
//!
//! When the user submits the composer text (Normal-mode `<CR>` or Insert-mode
//! `Alt+Enter`), the first character determines the action:
//! - `!` prefix  → strip it and run the remainder as a bash command.
//! - anything else → send as a prompt to the agent session.

use std::path::Path;

use tokio::sync::oneshot;

use crate::notepad::terminal;

/// Result of parsing a submitted line.
#[derive(Debug, PartialEq, Eq)]
pub enum SubmitKind {
    /// Non-empty text to send to the agent.
    Prompt(String),
    /// `!`-prefixed bash command (already stripped of `!`).
    Bash(String),
    /// Empty / whitespace-only — no action.
    None,
}

/// Parse raw composer text into a [`SubmitKind`].
///
/// Leading whitespace is stripped before checking for `!`. An empty or
/// whitespace-only result yields [`SubmitKind::None`].
pub fn parse_submit(text: &str) -> SubmitKind {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return SubmitKind::None;
    }
    if let Some(rest) = trimmed.strip_prefix('!') {
        let cmd = rest.trim();
        if cmd.is_empty() {
            return SubmitKind::None;
        }
        SubmitKind::Bash(cmd.to_string())
    } else {
        SubmitKind::Prompt(trimmed.to_string())
    }
}

/// Spawn a background `sh -c` command and return a receiver for its output.
///
/// The command runs with the same 10 s timeout semantics as
/// [`terminal::run_command`]. The caller polls the receiver with `try_recv`
/// to avoid blocking the TUI event loop.
pub fn spawn_bash(cmd: &str, workdir: &Path) -> oneshot::Receiver<String> {
    let (tx, rx) = oneshot::channel();
    let cmd_owned = cmd.to_string();
    let workdir_owned = workdir.to_path_buf();
    tokio::spawn(async move {
        let out = terminal::run_command(&cmd_owned, &workdir_owned).await;
        let _ = tx.send(out);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_prompt() {
        assert_eq!(
            parse_submit("hello world"),
            SubmitKind::Prompt("hello world".into())
        );
    }

    #[test]
    fn parse_prompt_with_leading_ws() {
        assert_eq!(parse_submit("  hi  "), SubmitKind::Prompt("hi".into()));
    }

    #[test]
    fn parse_bash() {
        assert_eq!(parse_submit("!ls -la"), SubmitKind::Bash("ls -la".into()));
    }

    #[test]
    fn parse_bash_with_leading_ws() {
        assert_eq!(
            parse_submit("  !echo hi"),
            SubmitKind::Bash("echo hi".into())
        );
    }

    #[test]
    fn parse_empty() {
        assert_eq!(parse_submit(""), SubmitKind::None);
    }

    #[test]
    fn parse_whitespace_only() {
        assert_eq!(parse_submit("   "), SubmitKind::None);
    }

    #[test]
    fn parse_bang_only() {
        assert_eq!(parse_submit("!"), SubmitKind::None);
    }

    #[test]
    fn parse_bang_whitespace() {
        assert_eq!(parse_submit("!   "), SubmitKind::None);
    }

    #[tokio::test]
    async fn spawn_bash_returns_output() {
        let tmp = tempfile::tempdir().unwrap();
        let rx = spawn_bash("echo hello", tmp.path());
        let out = rx.await.unwrap();
        assert!(out.contains("hello"));
    }
}
