//! Post-TUI exit tips: print a friendly hint about optional features that
//! are not yet set up (missing tmux, missing skill dependencies). Only shown
//! after the TUI exits, and only for TUI/ts command paths (never headless).

use std::process::Command;

/// True when a `tmux` binary is on PATH.
fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// True when the skills-deps sentinel file exists.
fn deps_sentinel_exists() -> bool {
    opencoder_core::skills_dir()
        .join(opencoder_core::DEPS_SENTINEL)
        .exists()
}

/// Print optional-feature tips to stderr after the TUI exits.
/// No-op when everything is already set up.
pub fn print_exit_tips() {
    let missing_tmux = !tmux_available();
    let missing_deps = !deps_sentinel_exists();

    if !missing_tmux && !missing_deps {
        return;
    }

    let mut lines = Vec::new();
    lines.push(String::from(""));
    lines.push(String::from(
        "\x1b[36m\x1b[1mTips: some optional features are not set up yet:\x1b[0m",
    ));
    lines.push(String::from(""));

    if missing_tmux {
        lines.push(String::from(
            "  \x1b[33m-tmux not installed\x1b[0m — need sessions that survive disconnects?",
        ));
        lines.push(String::from(
            "    Install: apt install tmux  -  then use: opencode ts (persistent TUI)",
        ));
        lines.push(String::from(""));
    }

    if missing_deps {
        lines.push(String::from(
            "  \x1b[33m-Optional skill deps not installed\x1b[0m — unlock 2 skills:",
        ));
        lines.push(String::from(
            "    - ssh-pty: persistent SSH sessions via send/read",
        ));
        lines.push(String::from(
            "    - chrome-headless: headless browser for JS-heavy pages + screenshots",
        ));
        lines.push(String::from(""));
        lines.push(String::from(
            "    Setup:  ~/.opencoder/install-skills-dep.sh",
        ));
        lines.push(String::from(
            "    Then in TUI: press $ and type the skill name to activate.",
        ));
        lines.push(String::from(""));
    }

    for line in &lines {
        eprintln!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_check_does_not_panic() {
        // Just verify the function doesn't panic regardless of environment.
        let _ = deps_sentinel_exists();
    }
}
