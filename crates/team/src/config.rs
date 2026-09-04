//! Run-scoped configuration and cooperative cancellation. Plain data +
//! free functions: `TeamRunConfig` narrows the global `Config` to the three
//! knobs the team runtime reads, `CancelToken` is a shared bool checked
//! between steps (no async cancellation magic — every check is explicit).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use opencoder_core::Config;

/// Plain data, `Clone` so the web layer can hand a copy to each spawned
/// topic runtime (the original stays in `AppState`).
#[derive(Clone)]
pub struct TeamRunConfig {
    pub team_root: PathBuf,
    pub max_turns: usize,
    pub max_sub_turns: usize,
}

impl From<&Config> for TeamRunConfig {
    fn from(config: &Config) -> Self {
        Self {
            team_root: config.team_root.clone(),
            max_turns: config.team_max_turns,
            max_sub_turns: config.team_max_sub_turns,
        }
    }
}

/// Cooperative cancellation, checked between steps.
#[derive(Clone)]
pub struct CancelToken(Arc<AtomicBool>);

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancelToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
