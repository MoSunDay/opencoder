//! `opencode ts` (alias `rs`) -- run the TUI inside a tmux session that
//! survives SSH disconnect. A bare `ts`/`rs` always starts a fresh managed
//! session; `ts -l`/`rs -l` lists them and `ts -r <id>`/`rs -r <id>` reattaches.
//!
//! tmux is engaged ONLY when `opencode ts`/`rs` is used. Plain `tui`, `run`,
//! headless and server commands are completely unaffected.
//!
//! Safety: every tmux argument is passed via `Command::arg(...)` -- tmux runs
//! the pane command with execvp, never a shell -- so session names cannot
//! inject shell metacharacters.
//!
//! Naming contract: a managed tmux session is named `opencode-<ulid>` where the
//! ulid is also a real opencode session id (seeded into the store). That gives
//! one stable id shared by tmux and the session store, so `ts -l` can show
//! `/task`-style info and `ts -r <id>` resolves unambiguously.

mod actions;
mod display;
mod env;
mod naming;
mod tmux;

use anyhow::Result;

use crate::Cli;

pub use env::{inside_tmux, tmux_available};

// ===== dispatch ============================================================

/// Entry point routed from `main.rs`.
pub async fn ts_dispatch(
    cli: &Cli,
    list: bool,
    resume: Option<&str>,
    clean: bool,
) -> Result<()> {
    if list {
        actions::ts_list(cli).await
    } else if let Some(id) = resume {
        actions::ts_resume(cli, id).await
    } else if clean {
        actions::ts_cleanup(cli).await
    } else {
        actions::ts_start(cli).await
    }
}

/// Decide whether `opencode ts` should run the TUI inline instead of engaging
/// tmux: only when not listing, not resuming, and already inside a tmux client
/// -- so we never nest tmux. Pure so the branching contract is unit-testable
/// without spawning tmux. Called by `main.rs`.
pub fn runs_inline(list: bool, has_resume: bool, inside: bool) -> bool {
    !list && !has_resume && inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_inline_only_when_inside_without_flags() {
        // The single inline case: inside tmux, with neither -l nor -r.
        assert!(runs_inline(false, false, true));
        // Listing must engage tmux even when already inside it.
        assert!(!runs_inline(true, false, true));
        // Resuming a specific session must engage tmux.
        assert!(!runs_inline(false, true, true));
        // Never inline when not already inside tmux (the whole point of `ts`).
        assert!(!runs_inline(false, false, false));
        assert!(!runs_inline(true, true, false));
    }
}
