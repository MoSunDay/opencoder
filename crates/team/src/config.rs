//! Run-scoped configuration and cooperative cancellation. Plain data +
//! free functions: `TeamRunConfig` narrows the global `Config` to the three
//! knobs the team runtime reads, `CancelToken` is a shared bool checked
//! between steps (no async cancellation magic — every check is explicit).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use opencoder_core::{validate_team_turn_budgets, Config};

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

impl TeamRunConfig {
    /// Refuse an invalid budget at the runtime boundary, including configs
    /// assembled directly by an embedding caller instead of `Config::load`.
    pub fn validate(&self) -> Result<()> {
        validate_team_turn_budgets(self.max_turns, self.max_sub_turns).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::TeamRunConfig;

    #[test]
    fn runtime_budget_boundary_is_fail_fast() {
        let root = std::path::PathBuf::from("/unused");
        for value in [1, 999] {
            assert!(TeamRunConfig {
                team_root: root.clone(),
                max_turns: value,
                max_sub_turns: value,
            }
            .validate()
            .is_ok());
        }
        for (max_turns, max_sub_turns) in [(0, 1), (1000, 1), (1, 0), (1, 1000)] {
            assert!(TeamRunConfig {
                team_root: root.clone(),
                max_turns,
                max_sub_turns,
            }
            .validate()
            .is_err());
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
