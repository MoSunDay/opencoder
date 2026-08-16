//! `opencoder install-tools`: detect the optional tools dependencies (tmux)
//! and, if missing, run the embedded `install-skills-dep.sh` with inherited
//! stdio (it needs an interactive TTY for the `sudo` password), then re-seed
//! the now-unlocked `ssh-pty` skill.
//!
//! Ported from the former TUI `/install_tools` slash command; the execution
//! logic (probe → installer → re-seed → report) is unchanged, only the
//! surface moved: plain stdout lines instead of chat markers, and no TUI
//! screen to suspend/resume.

use std::process::Command;

use opencoder_core::tool_deps::{ToolDepStatus, all_installed, check_tool_deps};
use opencoder_core::{seed_dep_gated_skills, skills_dir, write_install_script};

/// Path to the installer written into `~/.opencoder/` by
/// [`write_install_script`]. `skills_dir()` is `~/.opencoder/skills`, so its
/// parent is the install root — avoids a `dirs` dependency in the cli crate.
fn install_script_path() -> std::path::PathBuf {
    let root = skills_dir();
    match root.parent() {
        Some(p) => p.join("install-skills-dep.sh"),
        None => std::path::PathBuf::from("install-skills-dep.sh"),
    }
}

/// Orchestrator: detect deps, run the installer if needed (inherited stdio so
/// the user sees live output and can type a sudo password), then re-seed the
/// dep-gated skills and report. Returns the installer's exit code so `main`
/// can propagate it as the process exit status.
pub fn install_tools_run() -> anyhow::Result<i32> {
    let status = check_tool_deps();
    if all_installed(&status) {
        println!("[install_tools] tmux already installed \u{2014} nothing to do");
        return Ok(0);
    }

    // Ensure the script exists on disk (idempotent) before we try to run it.
    write_install_script();

    println!("[install_tools] running installer \u{2014} a sudo password may be required\u{2026}");

    let script = install_script_path();
    let exit_code = match Command::new(&script).status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("[install_tools] failed to launch installer: {e}");
            return Err(e.into());
        }
    };

    // Re-seed the dep-gated skills so ssh-pty / chrome-headless appear now.
    seed_dep_gated_skills();

    let new_status = check_tool_deps();
    for line in format_result(exit_code, &new_status) {
        println!("{line}");
    }
    Ok(exit_code)
}

/// Pure: build the result lines from the installer exit code + the new
/// dependency status. Three lines on success (head, status row, tail); two on
/// failure (head, status row) since the success/warning tail is meaningless.
fn format_result(exit_code: i32, status: &ToolDepStatus) -> Vec<String> {
    let ok = exit_code == 0;
    let mut lines = Vec::new();
    lines.push(format!(
        "[install_tools] installer exited with code {exit_code}"
    ));
    lines.push(format!(
        "  tmux: {}  |  sentinel: {}",
        yn(status.tmux),
        yn(status.sentinel),
    ));
    if ok && all_installed(status) {
        lines.push(String::from(
            "  all tools deps installed \u{2014} ssh-pty skill unlocked",
        ));
    } else if ok {
        lines.push(String::from(
            "  some deps still missing \u{2014} re-run opencode install-tools or install manually",
        ));
    }
    lines
}

fn yn(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_result_success_all_installed() {
        let status = ToolDepStatus {
            tmux: true,
            sentinel: true,
        };
        let lines = format_result(0, &status);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("exited with code 0"));
        assert!(lines[1].contains("tmux: yes"));
        assert!(lines[2].contains("all tools deps installed"));
    }

    #[test]
    fn format_result_success_but_still_missing() {
        let status = ToolDepStatus {
            tmux: false,
            sentinel: false,
        };
        let lines = format_result(0, &status);
        assert_eq!(lines.len(), 3);
        assert!(lines[1].contains("tmux: no") && lines[1].contains("sentinel: no"));
        assert!(lines[2].contains("some deps still missing"));
    }

    #[test]
    fn format_result_failure_has_no_tail() {
        let status = ToolDepStatus {
            tmux: false,
            sentinel: false,
        };
        let lines = format_result(2, &status);
        assert_eq!(lines.len(), 2, "non-zero exit -> only head + status row");
        assert!(lines[0].contains("exited with code 2"));
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
