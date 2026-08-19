//! Tool-dependency status: detect whether the optional tools dependency
//! (tmux) is installed, plus whether the `install-skills-dep.sh` sentinel
//! exists.
//!
//! Shared probe logic so the CLI exit-tips and the TUI `/install_tools`
//! command agree on what "installed" means.

use std::process::Command;

use crate::skill::{skills_dir, DEPS_SENTINEL};

/// Snapshot of which optional tool dependencies are present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToolDepStatus {
    /// `tmux -V` exits successfully.
    pub tmux: bool,
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

/// Probe the current environment for the optional tool dependencies.
/// No home directory → no sentinel (nothing was ever seeded there).
pub fn check_tool_deps() -> ToolDepStatus {
    ToolDepStatus {
        tmux: tmux_available(),
        sentinel: skills_dir().is_some_and(|d| d.join(DEPS_SENTINEL).exists()),
    }
}

/// True when every optional dependency is present.
pub fn all_installed(s: &ToolDepStatus) -> bool {
    s.tmux && s.sentinel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_installed_truth_table() {
        let mk = |tmux, sentinel| ToolDepStatus { tmux, sentinel };
        // All true -> installed.
        assert!(all_installed(&mk(true, true)));
        // Any single false -> not installed.
        assert!(!all_installed(&mk(false, true)));
        assert!(!all_installed(&mk(true, false)));
        // All false -> not installed.
        assert!(!all_installed(&mk(false, false)));
    }

    #[test]
    fn default_status_is_not_installed() {
        assert!(!all_installed(&ToolDepStatus::default()));
    }
}
