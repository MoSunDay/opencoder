//! Environment detection: is tmux installed / are we inside a tmux client?

use std::path::PathBuf;

/// True when a `tmux` executable is on PATH.
pub fn tmux_available() -> bool {
    which_tmux().is_some()
}

pub(crate) fn which_tmux() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("tmux");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// True when already running inside a tmux client (`TMUX` is set by tmux for
/// every pane). Mirrors `crates/tui/src/selection.rs`.
pub fn inside_tmux() -> bool {
    std::env::var_os("TMUX").is_some()
}
