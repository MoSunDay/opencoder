//! `/install_tools`: detect the optional tools dependencies (tmux)
//! and, if missing, suspend the TUI so the embedded
//! `install-skills-dep.sh` can run with inherited stdio (it needs an
//! interactive TTY for the `sudo` password), then resume the TUI and re-seed
//! the now-unlocked `ssh-pty` skill.

use std::process::Command;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use opencoder_core::tool_deps::{all_installed, check_tool_deps, ToolDepStatus};
use opencoder_core::{seed_dep_gated_skills, skills_dir, write_install_script};

use crate::chat::ChatView;
use crate::render::Term;
use crate::terminal;
use crate::theme;

/// Path to the installer written into `~/.opencoder/` by
/// [`write_install_script`]. `skills_dir()` is `~/.opencoder/skills`, so its
/// parent is the install root — avoids a `dirs` dependency in the tui crate.
fn install_script_path() -> std::path::PathBuf {
    let root = skills_dir();
    match root.parent() {
        Some(p) => p.join("install-skills-dep.sh"),
        None => std::path::PathBuf::from("install-skills-dep.sh"),
    }
}

/// Orchestrator: detect deps, run the installer if needed, resume the screen,
/// then re-seed the dep-gated skills. Pushes marker lines into `chat` for
/// every outcome so the user sees what happened.
pub(crate) fn run(terminal: &mut Term, chat: &mut ChatView) {
    let status = check_tool_deps();
    if all_installed(&status) {
        chat.push_marker(Line::from(Span::styled(
            "[install_tools] tmux already installed \u{2014} nothing to do",
            Style::default().fg(theme::ok_color()),
        )));
        return;
    }

    // Ensure the script exists on disk (idempotent) before we try to run it.
    write_install_script();

    chat.push_marker(Line::from(Span::styled(
        "[install_tools] running installer \u{2014} see the terminal below \
         (a sudo password may be required)\u{2026}",
        Style::default().fg(theme::local_color()),
    )));

    let exit_code = match suspend_and_run() {
        Ok(code) => code,
        Err(e) => {
            // Best-effort resume even on failure so the TUI is not left raw-less.
            let _ = terminal::resume_screen();
            let _ = terminal.clear();
            chat.push_marker(Line::from(Span::styled(
                format!("[install_tools] failed to launch installer: {e}"),
                Style::default().fg(theme::err_color()),
            )));
            return;
        }
    };

    // Resume the TUI screen, then invalidate ratatui's diff buffer so the next
    // draw is a full repaint (the alt screen was torn down and rebuilt).
    let _ = terminal::resume_screen();
    let _ = terminal.clear();

    // Re-seed the dep-gated skills so ssh-pty / chrome-headless appear now.
    seed_dep_gated_skills();

    let new_status = check_tool_deps();
    chat.push_marker_lines(format_result(exit_code, &new_status));
}

/// Suspend the TUI screen, run the installer with inherited stdio, return its
/// exit code. The screen is left suspended on return; the caller resumes it.
fn suspend_and_run() -> anyhow::Result<i32> {
    terminal::suspend_screen()?;
    let script = install_script_path();
    // Inherited stdio so the user sees live output and can type a sudo password.
    let status = Command::new(&script).status()?;
    Ok(status.code().unwrap_or(1))
}

/// Pure: build the result marker lines from the installer exit code + the new
/// dependency status. Three lines on success (head, status row, tail); two on
/// failure (head, status row) since the success/warning tail is meaningless.
fn format_result(exit_code: i32, status: &ToolDepStatus) -> Vec<Line<'static>> {
    let ok = exit_code == 0;
    let head_color = if ok {
        theme::ok_color()
    } else {
        theme::err_color()
    };
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("[install_tools] installer exited with code {exit_code}"),
        Style::default().fg(head_color),
    )));
    lines.push(Line::from(Span::styled(
        format!(
            "  tmux: {}  |  sentinel: {}",
            yn(status.tmux),
            yn(status.sentinel),
        ),
        Style::default().fg(theme::local_color()),
    )));
    if ok && all_installed(status) {
        lines.push(Line::from(Span::styled(
            "  all tools deps installed \u{2014} ssh-pty skill \
             unlocked (press $ to activate)",
            Style::default().fg(theme::ok_color()),
        )));
    } else if ok {
        lines.push(Line::from(Span::styled(
            "  some deps still missing \u{2014} re-run /install_tools or install manually",
            Style::default().fg(theme::warn_color()),
        )));
    }
    lines
}

fn yn(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_strings(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| &*s.content).collect::<String>())
            .collect()
    }

    #[test]
    fn format_result_success_all_installed() {
        let status = ToolDepStatus {
            tmux: true,
            sentinel: true,
        };
        let lines = format_result(0, &status);
        assert_eq!(lines.len(), 3);
        let r = to_strings(&lines);
        assert!(r[0].contains("exited with code 0"));
        assert!(r[1].contains("tmux: yes"));
        assert!(r[2].contains("all tools deps installed"));
    }

    #[test]
    fn format_result_success_but_still_missing() {
        let status = ToolDepStatus {
            tmux: false,
            sentinel: false,
        };
        let lines = format_result(0, &status);
        assert_eq!(lines.len(), 3);
        let r = to_strings(&lines);
        assert!(r[1].contains("tmux: no") && r[1].contains("sentinel: no"));
        assert!(r[2].contains("some deps still missing"));
    }

    #[test]
    fn format_result_failure_has_no_tail() {
        let status = ToolDepStatus {
            tmux: false,
            sentinel: false,
        };
        let lines = format_result(2, &status);
        assert_eq!(lines.len(), 2, "non-zero exit -> only head + status row");
        let r = to_strings(&lines);
        assert!(r[0].contains("exited with code 2"));
    }

    #[test]
    fn format_result_failure_even_if_deps_present() {
        let status = ToolDepStatus {
            tmux: true,
            sentinel: true,
        };
        let lines = format_result(1, &status);
        assert_eq!(lines.len(), 2, "non-zero exit suppresses the tail line");
    }
}
