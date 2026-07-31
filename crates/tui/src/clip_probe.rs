//! Terminal clipboard capability probe + smart local-command dispatch.
//!
//! VTE-based terminals (GNOME Terminal, Terminator, Xfce, MATE) and GNU screen
//! silently discard OSC 52 clipboard sequences by default. This module probes
//! the environment to classify the terminal, then dispatches local clipboard
//! commands (xclip/xsel/wl-copy/pbcopy/clip.exe) with awareness of the display
//! server (Wayland vs X11 vs headless) and SSH context.
//!
//! The classification is a **pure function** that accepts an env-var accessor
//! closure, making it fully testable without touching the real environment.
//! [`probe_clipboard`] memoises the result via `OnceLock` so repeated copies do
//! not re-probe.

use std::sync::OnceLock;
use std::time::Duration;

/// Snapshot of the terminal/display environment relevant to clipboard copy.
/// Plain data - no methods; all logic lives in free functions below.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClipProbe {
    /// True for VTE-based terminals (GNOME, Terminator, Xfce, MATE, Guake,
    /// Tilix) that silently drop OSC 52 by default.
    pub is_vte: bool,
    /// True when running inside GNU screen (`$STTY` / `$STY` set).
    pub is_screen: bool,
    /// True when running inside tmux (`$TMUX` set).
    pub is_tmux: bool,
    /// True when this looks like an SSH session (`$SSH_CONNECTION` /
    /// `$SSH_TTY`).
    pub is_ssh: bool,
    /// Whether OSC 52 can be trusted to actually reach the clipboard. This is
    /// `true` only for positively-identified reliable terminals; anything
    /// unknown is conservatively treated as `false`.
    pub osc52_reliable: bool,
    /// True when `$WAYLAND_DISPLAY` is set.
    pub wayland: bool,
    /// True when `$DISPLAY` is set (X11 / XWayland).
    pub x11: bool,
}

// ── Terminal fingerprints ──────────────────────────────────────────

/// Substrings indicating a VTE-based terminal when found in `TERM_PROGRAM`,
/// `COLORTERM`, or `TERM`.
const VTE_HINTS: &[&str] = &[
    "gnome", "terminator", "xfce", "mate", "vte", "guake", "tilix",
    "GNOME", "Terminator", "Xfce", "MATE", "VTE", "Guake", "Tilix",
];

/// Substrings identifying terminals known to honour OSC 52 out-of-the-box.
const RELIABLE_HINTS: &[&str] = &[
    "iTerm.app",
    "WezTerm",
    "wezterm",
    "alacritty",
    "Alacritty",
    "kitty",
    "Kitty",
    "ghostty",
    "Ghostty",
    "foot",
];

/// Check whether any of `VTE_HINTS` appears in the given env-var values.
fn is_vte_terminal(term_program: &str, colorterm: &str, term: &str) -> bool {
    VTE_HINTS.iter().any(|hint| {
        term_program.contains(hint) || colorterm.contains(hint) || term.contains(hint)
    })
}

/// Check whether the terminal is positively identified as OSC-52-reliable.
/// VTE and screen are excluded even if they also match a reliable hint
/// (belt-and-suspenders).
fn osc52_is_reliable(
    term_program: &str,
    term: &str,
    is_vte: bool,
    is_screen: bool,
    wt_session: bool,
) -> bool {
    if is_vte || is_screen {
        return false;
    }
    wt_session
        || RELIABLE_HINTS
            .iter()
            .any(|h| term_program.contains(h) || term.contains(h))
}

/// Classify the terminal and display environment. Pure function: takes a
/// closure that reads environment variables, so callers can inject test data.
fn classify_terminal(get_var: impl Fn(&str) -> Option<String>) -> ClipProbe {
    let term_program = get_var("TERM_PROGRAM").unwrap_or_default();
    let colorterm = get_var("COLORTERM").unwrap_or_default();
    let term = get_var("TERM").unwrap_or_default();
    let wt_session = get_var("WT_SESSION").is_some();

    let is_tmux = get_var("TMUX").is_some();
    let is_screen = get_var("STY").is_some();
    let is_ssh = get_var("SSH_CONNECTION").is_some() || get_var("SSH_TTY").is_some();
    let wayland = get_var("WAYLAND_DISPLAY").is_some();
    let x11 = get_var("DISPLAY").is_some();

    let is_vte = is_vte_terminal(&term_program, &colorterm, &term);
    let osc52_reliable = osc52_is_reliable(&term_program, &term, is_vte, is_screen, wt_session);

    ClipProbe {
        is_vte,
        is_screen,
        is_tmux,
        is_ssh,
        osc52_reliable,
        wayland,
        x11,
    }
}

/// Probe the real environment and cache the result. Subsequent calls return
/// the cached [`ClipProbe`] without re-reading environment variables.
pub fn probe_clipboard() -> ClipProbe {
    static CACHE: OnceLock<ClipProbe> = OnceLock::new();
    CACHE
        .get_or_init(|| classify_terminal(|k| std::env::var(k).ok()))
        .clone()
}

// ── Local clipboard command dispatch ───────────────────────────────

/// Maximum time to wait for a single local clipboard command before giving up
/// and killing it. Generous enough for the slowest reasonable command (e.g.
/// `xclip` initialising an X connection) yet short enough that a hung helper
/// never blocks for long.
const CLIP_CMD_TIMEOUT: Duration = Duration::from_secs(3);

/// Spawn `prog` with `args`, write `input` to its stdin, and wait for it to
/// exit - but no longer than [`CLIP_CMD_TIMEOUT`]. Returns `Some(())` only
/// when the program was found *and* exited successfully within the deadline;
/// `None` otherwise (missing binary, non-zero exit, timeout, I/O error). On
/// timeout the child is killed. Never panics.
pub(crate) fn try_spawn(prog: &str, args: &[&str], input: &str) -> Option<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::Instant;
    let mut child = Command::new(prog)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
        // `stdin` drops here, closing the pipe and signalling EOF so the child
        // can finish reading.
    }
    // Poll instead of a blocking `wait()`: a clipboard helper that hangs (e.g.
    // `xclip` against an unresponsive X server) would otherwise block the
    // calling thread indefinitely.
    let deadline = Instant::now() + CLIP_CMD_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() { Some(()) } else { None };
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // reap the zombie after kill
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}

/// Copy `text` via a platform-native clipboard command, choosing candidates
/// based on the probed display server. On SSH sessions local commands are
/// skipped entirely (they would write to the *remote* clipboard). Returns the
/// name of the tool that succeeded, or `None`.
pub fn copy_local_smart(probe: &ClipProbe, text: &str) -> Option<&'static str> {
    // SSH: local commands write to the *remote* clipboard, which is almost
    // never what the user wants. Skip them entirely.
    if probe.is_ssh {
        return None;
    }

    #[cfg(target_os = "macos")]
    {
        if try_spawn("pbcopy", &[], text).is_some() {
            return Some("pbcopy");
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Wayland: prefer wl-copy; fall back to X11 tools (XWayland bridge).
        if probe.wayland && try_spawn("wl-copy", &[], text).is_some() {
            return Some("wl-copy");
        }
        // xclip / xsel work on both X11 and XWayland.
        if probe.wayland || probe.x11 {
            if try_spawn("xclip", &["-selection", "clipboard"], text).is_some() {
                return Some("xclip");
            }
            if try_spawn("xsel", &["--clipboard", "--input"], text).is_some() {
                return Some("xsel");
            }
        }
        // No display server at all (headless): no local clipboard command.
    }

    #[cfg(target_os = "windows")]
    {
        if try_spawn("clip.exe", &[], text).is_some() {
            return Some("clip.exe");
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = text;
    }

    None
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Convenience: classify from a list of (key, value) pairs.
    fn classify(vars: &[(&str, &str)]) -> ClipProbe {
        classify_terminal(|k| {
            vars.iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.to_string())
        })
    }

    #[test]
    fn detects_vte_via_term_program() {
        let p = classify(&[("TERM_PROGRAM", "gnome-terminal")]);
        assert!(p.is_vte);
        assert!(!p.osc52_reliable);
    }

    #[test]
    fn detects_terminator() {
        let p = classify(&[("COLORTERM", "terminator-dark")]);
        assert!(p.is_vte);
        assert!(!p.osc52_reliable);
    }

    #[test]
    fn detects_xfce_terminal() {
        let p = classify(&[("TERM", "xfce4-terminal")]);
        assert!(p.is_vte);
    }

    #[test]
    fn detects_screen_via_sty() {
        let p = classify(&[("STY", "12345.pts-0.host")]);
        assert!(p.is_screen);
        assert!(!p.osc52_reliable);
    }

    #[test]
    fn detects_tmux() {
        let p = classify(&[("TMUX", "/tmp/tmux-0/default,1234,0")]);
        assert!(p.is_tmux);
    }

    #[test]
    fn detects_ssh_via_ssh_connection() {
        let p = classify(&[("SSH_CONNECTION", "10.0.0.1 1234 10.0.0.2 22")]);
        assert!(p.is_ssh);
    }

    #[test]
    fn detects_ssh_via_ssh_tty() {
        let p = classify(&[("SSH_TTY", "/dev/pts/1")]);
        assert!(p.is_ssh);
    }

    #[test]
    fn reliable_iterm2() {
        let p = classify(&[("TERM_PROGRAM", "iTerm.app")]);
        assert!(!p.is_vte);
        assert!(p.osc52_reliable);
    }

    #[test]
    fn reliable_wezterm() {
        let p = classify(&[("TERM_PROGRAM", "WezTerm")]);
        assert!(p.osc52_reliable);
    }

    #[test]
    fn reliable_alacritty_via_term() {
        // Alacritty may not set TERM_PROGRAM but sets TERM=alacritty.
        let p = classify(&[("TERM", "alacritty")]);
        assert!(p.osc52_reliable);
    }

    #[test]
    fn reliable_kitty() {
        let p = classify(&[("TERM", "kitty")]);
        assert!(p.osc52_reliable);
    }

    #[test]
    fn reliable_windows_terminal_via_wt_session() {
        let p = classify(&[("WT_SESSION", "1")]);
        assert!(p.osc52_reliable);
    }

    #[test]
    fn unknown_terminal_is_conservatively_unreliable() {
        // Generic TERM with no identifiable terminal -> treat as unreliable.
        let p = classify(&[("TERM", "xterm-256color")]);
        assert!(!p.osc52_reliable);
        assert!(!p.is_vte);
    }

    #[test]
    fn wayland_detection() {
        let p = classify(&[
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("DISPLAY", ":0"),
        ]);
        assert!(p.wayland);
        assert!(p.x11);
    }

    #[test]
    fn x11_only_detection() {
        let p = classify(&[("DISPLAY", ":0")]);
        assert!(!p.wayland);
        assert!(p.x11);
    }

    #[test]
    fn headless_detection() {
        let p = classify(&[]);
        assert!(!p.wayland);
        assert!(!p.x11);
        assert!(!p.osc52_reliable);
    }

    #[test]
    fn try_spawn_missing_program_returns_none() {
        assert!(try_spawn("opencoder-not-a-real-clipboard-bin-zz", &[], "").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn try_spawn_existing_program_succeeds_and_false_fails() {
        assert!(try_spawn("true", &[], "").is_some());
        assert!(try_spawn("false", &[], "").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn try_spawn_times_out_on_long_running_command() {
        let start = std::time::Instant::now();
        let result = try_spawn("sleep", &["30"], "");
        assert!(result.is_none(), "expected timeout -> None, got {result:?}");
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(20),
            "timed-out command should return well under 30 s, took {elapsed:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn copy_local_skips_on_ssh() {
        let probe = ClipProbe {
            is_ssh: true,
            x11: true,
            ..Default::default()
        };
        // Even on X11, SSH skips local commands.
        assert_eq!(copy_local_smart(&probe, "test"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn copy_local_skips_on_headless() {
        let probe = ClipProbe::default(); // no display
        assert_eq!(copy_local_smart(&probe, "test"), None);
    }
}
