//! `opencode ts` (alias `rs`) -- run the TUI inside a tmux session that
//! survives SSH disconnect. A bare `ts`/`rs` always starts a fresh managed
//! session; `ts -l` lists them **globally**, `ts -r <id>` reattaches, and
//! `ts -d <id>` removes one exact managed session.
//!
//! `ts -l` is tmux-first and global: every live managed tmux session from
//! every workdir is listed with its real workdir path (`pane_current_path`,
//! `$HOME` abbreviated to `~`), enriched with `/task` info from the central ts
//! registry (`<data_root>/ts.db`); stopped sessions come from that registry
//! (one indexed query — no per-store scan), but only when they were actually
//! started — plain `tui`/`run` sessions and never-started empty seeds are
//! never listed.
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
//! `/task`-style info and `ts -r <id>` resolves unambiguously. Each store also
//! carries a canonical `workdir` marker, so stopped sessions keep their path
//! and can be resumed globally from any directory.

mod actions;
mod display;
mod env;
mod naming;
mod registry;
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
    delete: Option<&str>,
) -> Result<()> {
    if list {
        actions::ts_list(cli).await
    } else if let Some(id) = resume {
        actions::ts_resume(cli, id).await
    } else if clean {
        actions::ts_cleanup(cli).await
    } else if let Some(id) = delete {
        actions::ts_delete(id).await
    } else {
        actions::ts_start(cli).await
    }
}
