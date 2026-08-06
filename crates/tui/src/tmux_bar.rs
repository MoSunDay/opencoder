//! tmux bottom status-bar management.
//!
//! Hides the tmux status bar for the duration of a TUI run (it eats a screen
//! row that competes with the full-screen TUI) and restores the previous state
//! on exit. This is a pure runtime override — no config file is read or written
//! and tmux is left exactly as it was once opencoder exits. No effect outside
//! tmux.

use std::process::Command;

/// True when running inside a tmux client (`TMUX` is set by every pane).
fn inside_tmux() -> bool {
    std::env::var_os("TMUX").is_some()
}

/// Parse a tmux `status` option value into a bool. `on` → Some(true),
/// `off` → Some(false), anything else (empty / unknown) → None ("don't
/// restore"). Surrounding whitespace is trimmed because `tmux display-message`
/// emits a trailing newline on stdout.
fn parse_status(raw: &str) -> Option<bool> {
    match raw.trim() {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

/// Reads the current `status` option via `tmux display-message -p`. Returns
/// `None` when tmux is unavailable or the query fails ("don't restore").
fn current_status() -> Option<bool> {
    let out = Command::new("tmux")
        .args(["display-message", "-p", "#{status}"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_status(&String::from_utf8_lossy(&out.stdout))
}

/// Best-effort `tmux set status on|off`. Errors are swallowed: this is purely
/// cosmetic, never worth aborting startup over.
fn set_status(on: bool) {
    let _ = Command::new("tmux")
        .args(["set", "status", if on { "on" } else { "off" }])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Capture the live status bar state and hide it. Returns the captured state
/// to hand to [`restore`]. No-op when not inside tmux.
pub fn hide() -> Option<bool> {
    if !inside_tmux() {
        return None;
    }
    let prev = current_status();
    set_status(false);
    prev
}

/// Restore the status bar to the state captured by [`hide`]. `None` is a no-op
/// (was not inside tmux, or state was unreadable).
pub fn restore(prev: Option<bool>) {
    if let Some(on) = prev {
        set_status(on);
    }
}

// Note (rules/01 I/O-wrapper exemption): `current_status` / `set_status` /
// `inside_tmux` shell out to the `tmux` binary or read the process environment.
// They are pure I/O wrappers that cannot run in a tmux-less CI sandbox, so they
// are not unit-tested directly. The only business logic — parsing the status
// string — is extracted into `parse_status` and fully covered below.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_recognizes_on() {
        assert_eq!(parse_status("on"), Some(true));
    }

    #[test]
    fn parse_status_recognizes_off() {
        assert_eq!(parse_status("off"), Some(false));
    }

    #[test]
    fn parse_status_rejects_unknown_value() {
        // tmux could emit anything unexpected — never restore blindly.
        assert_eq!(parse_status("weird"), None);
        assert_eq!(parse_status(""), None);
    }

    #[test]
    fn parse_status_trims_trailing_newline() {
        // `tmux display-message -p` appends a trailing newline to stdout, and
        // may include surrounding spaces; trim must handle it.
        assert_eq!(parse_status("on\n"), Some(true));
        assert_eq!(parse_status("  off \n"), Some(false));
    }

    #[test]
    fn hide_returns_none_outside_tmux() {
        // Outside tmux, hide() must short-circuit before spawning any process
        // and return None so restore() is a later no-op. TMUX is process-global,
        // so save/restore it to avoid perturbing parallel tests on a dev box.
        let saved = std::env::var_os("TMUX");
        std::env::remove_var("TMUX");
        let result = hide();
        if let Some(v) = saved {
            std::env::set_var("TMUX", v);
        }
        assert_eq!(result, None);
    }

    #[test]
    fn restore_none_is_noop() {
        // restore(None) must not panic and must not spawn a tmux process.
        restore(None);
    }
}
