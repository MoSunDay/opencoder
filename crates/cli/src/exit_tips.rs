//! Post-TUI exit tips: print a friendly hint about optional features that
//! are not yet set up (missing tmux, missing skill dependencies). Only shown
//! after the TUI exits, and only for TUI/ts command paths (never headless).

/// Print optional-feature tips to stderr after the TUI exits.
/// No-op when everything is already set up.
pub fn print_exit_tips() {
    let status = opencoder_core::check_tool_deps();
    let missing_tmux = !status.tmux;
    let missing_deps = !status.sentinel;

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
            "  \x1b[33m-Optional skill deps not installed\x1b[0m — unlock 1 skill:",
        ));
        lines.push(String::from(
            "    - ssh-pty: persistent SSH sessions via send/read",
        ));
        lines.push(String::from(""));
        lines.push(String::from(
            "    Setup:  opencode install-tools",
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
    #[test]
    fn sentinel_check_does_not_panic() {
        // Just verify the shared probe doesn't panic regardless of environment.
        let _ = opencoder_core::check_tool_deps();
    }
}
