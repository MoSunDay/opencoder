//! Tool-dependency status: detect whether the optional tools dependencies
//! (tmux + a chromium-family browser) are installed, plus whether the
//! `install-skills-dep.sh` sentinel exists.
//!
//! Shared probe logic so the CLI exit-tips and the TUI `/install_tools`
//! command agree on what "installed" means. The chromium probe mirrors
//! `session::tools::chrome_headless::find_chrome` exactly (same
//! `$CHROME_PATH` + candidate-name scan) so a browser usable by the
//! headless-chrome tool is also considered installed here.

use std::path::PathBuf;
use std::process::Command;

use crate::skill::{skills_dir, DEPS_SENTINEL};

/// Standard chromium-family binary names searched on `$PATH`.
const CHROME_CANDIDATES: &[&str] = &[
    "google-chrome-stable",
    "google-chrome",
    "chromium-browser",
    "chromium",
];

/// Snapshot of which optional tool dependencies are present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToolDepStatus {
    /// `tmux -V` exits successfully.
    pub tmux: bool,
    /// A chromium-family binary is discoverable (mirrors `find_chrome`).
    pub chrome: bool,
    /// The `install-skills-dep.sh` sentinel file exists in `skills_dir()`.
    pub sentinel: bool,
}

/// True when `tmux` is on PATH and `tmux -V` exits successfully.
fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Verbatim mirror of `session::tools::chrome_headless::find_chrome`: checks
/// `$CHROME_PATH` first, then the standard candidate names on `$PATH`.
fn find_chrome() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CHROME_PATH") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in CHROME_CANDIDATES {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Probe the current environment for the optional tool dependencies.
pub fn check_tool_deps() -> ToolDepStatus {
    ToolDepStatus {
        tmux: tmux_available(),
        chrome: find_chrome().is_some(),
        sentinel: skills_dir().join(DEPS_SENTINEL).exists(),
    }
}

/// True when every optional dependency is present.
pub fn all_installed(s: &ToolDepStatus) -> bool {
    s.tmux && s.chrome && s.sentinel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_installed_truth_table() {
        let mk = |tmux, chrome, sentinel| ToolDepStatus {
            tmux,
            chrome,
            sentinel,
        };
        // All true -> installed.
        assert!(all_installed(&mk(true, true, true)));
        // Any single false -> not installed.
        assert!(!all_installed(&mk(false, true, true)));
        assert!(!all_installed(&mk(true, false, true)));
        assert!(!all_installed(&mk(true, true, false)));
        // All false -> not installed.
        assert!(!all_installed(&mk(false, false, false)));
        // Two-of-three combos -> not installed.
        assert!(!all_installed(&mk(false, false, true)));
        assert!(!all_installed(&mk(false, true, false)));
        assert!(!all_installed(&mk(true, false, false)));
    }

    #[test]
    fn default_status_is_not_installed() {
        assert!(!all_installed(&ToolDepStatus::default()));
    }
}
