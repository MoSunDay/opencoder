//! tmux `mouse` option coordination for native text selection.
//!
//! Inside tmux the `mouse` option makes tmux intercept drags *before* the
//! terminal emulator ever sees them, so native text selection (the whole
//! point of copy/selection mode) is impossible. This module turns `mouse`
//! off while copy/selection mode is active.
//!
//! The copy-mode flow intentionally does **not** restore the previous `mouse`
//! value on exit: keeping it off avoids the terminal/tmux tug-of-war that made
//! selection unreliable in the first place. The previous value is still
//! returned by [`disable`] for any caller that wishes to restore it manually.

use std::process::Command;

/// True when running inside a tmux client (`TMUX` is set by every pane).
fn inside_tmux() -> bool {
    std::env::var_os("TMUX").is_some()
}

/// Parse a tmux `mouse` option value. `on` -> Some(true), `off` -> Some(false),
/// anything else (empty / unknown / legacy numeric) -> None. Whitespace is
/// trimmed because `tmux show-options` emits a trailing newline.
fn parse_mouse(raw: &str) -> Option<bool> {
    match raw.trim() {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

/// Read the current `mouse` option via `tmux show-options -gv mouse`. Returns
/// `None` when tmux is unavailable or the query fails ("don't restore").
fn current_mouse() -> Option<bool> {
    let out = Command::new("tmux")
        .args(["show-options", "-gv", "mouse"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_mouse(&String::from_utf8_lossy(&out.stdout))
}

/// Best-effort `tmux set mouse on|off`. Errors are swallowed: purely cosmetic.
fn set_mouse(on: bool) {
    let _ = Command::new("tmux")
        .args(["set", "mouse", if on { "on" } else { "off" }])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Capture the live `mouse` state and turn it off. Returns the captured state
/// for optional manual restoration via [`restore`]. No-op outside tmux.
pub fn disable() -> Option<bool> {
    if !inside_tmux() {
        return None;
    }
    let prev = current_mouse();
    set_mouse(false);
    prev
}

/// Best-effort restore of a previously captured `mouse` state. `None` is a
/// no-op (was outside tmux, or state was unreadable).
pub fn restore(prev: Option<bool>) {
    if let Some(on) = prev {
        set_mouse(on);
    }
}

// Note (rules/01 I/O-wrapper exemption): `current_mouse` / `set_mouse` /
// `inside_tmux` shell out to the `tmux` binary or read the process
// environment. They are pure I/O wrappers that cannot run in a tmux-less CI
// sandbox, so they are not unit-tested directly. The only business logic --
// parsing the mouse string -- is extracted into `parse_mouse` and covered
// below.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mouse_recognizes_on() {
        assert_eq!(parse_mouse("on"), Some(true));
    }

    #[test]
    fn parse_mouse_recognizes_off() {
        assert_eq!(parse_mouse("off"), Some(false));
    }

    #[test]
    fn parse_mouse_rejects_unknown_value() {
        assert_eq!(parse_mouse("weird"), None);
        assert_eq!(parse_mouse(""), None);
        // Legacy tmux emitted numeric mouse modes (0/1/2); never restore blind.
        assert_eq!(parse_mouse("1"), None);
    }

    #[test]
    fn parse_mouse_trims_trailing_newline() {
        assert_eq!(parse_mouse("on\n"), Some(true));
        assert_eq!(parse_mouse("  off \n"), Some(false));
    }

    #[test]
    fn disable_returns_none_outside_tmux() {
        // Outside tmux, disable() must short-circuit before spawning any
        // process and return None. TMUX is process-global, so save/restore it.
        let saved = std::env::var_os("TMUX");
        std::env::remove_var("TMUX");
        let result = disable();
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
