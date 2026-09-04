//! Web-side runtime dependencies of the team feature, grouped so
//! [`crate::AppState`] stays one `Arc` field: the resolved
//! [`TeamRunConfig`] (team_root + turn bounds), the dispatcher the HTTP
//! layer fans prompts through (`NodeDispatcher` in production, a scripted
//! `MockDispatcher` in tests) and the [`TeamHub`] of live topic runtimes.

use std::path::Path;
use std::sync::Arc;

use opencoder_core::{data_dir_for, Config};
use opencoder_store::Store;
use opencoder_team::{NodeDispatcher, TeamDispatcher, TeamRunConfig};

use crate::team_hub::TeamHub;

pub struct TeamWebState {
    /// `team_root` on the shared team area + turn bounds from config.
    pub run: TeamRunConfig,
    /// Prompt fan-out to worker nodes (`NodeDispatcher` in production).
    pub dispatcher: Arc<dyn TeamDispatcher>,
    /// Live topic-runtime tasks keyed by topic id.
    pub hub: TeamHub,
}

impl TeamWebState {
    pub fn new(run: TeamRunConfig, dispatcher: Arc<dyn TeamDispatcher>) -> Self {
        TeamWebState {
            run,
            dispatcher,
            hub: TeamHub::new(),
        }
    }
}

/// Narrow a full [`Config`] to the three team knobs, rebasing an UNSET
/// `team_root` onto this workdir's data dir (`<data_dir>/team`) so the team
/// share always lives in the same directory tree as the store's database —
/// one backup/cleanup unit, no accidental writes into `data_local` root.
/// A root the user set explicitly (config file or env) is kept verbatim.
fn run_config_from(workdir: &Path, cfg: &Config) -> TeamRunConfig {
    let team_root = if cfg.team_root == Config::default().team_root {
        data_dir_for(workdir).join("team")
    } else {
        cfg.team_root.clone()
    };
    TeamRunConfig {
        team_root,
        max_turns: cfg.team_max_turns,
        max_sub_turns: cfg.team_max_sub_turns,
    }
}

/// Resolve the production team state for `workdir`: full config load (files
/// and env, same entry `serve`'s brain wiring uses); a load failure degrades
/// to defaults rather than refusing to boot.
pub fn production(store: Arc<dyn Store>, workdir: &Path) -> Arc<TeamWebState> {
    let cfg = Config::load(workdir).unwrap_or_else(|error| {
        tracing::warn!(error = %error, "config load failed; team falls back to defaults");
        Config::default()
    });
    Arc::new(TeamWebState::new(
        run_config_from(workdir, &cfg),
        Arc::new(NodeDispatcher::new(store)),
    ))
}

/// Test helper: scripted dispatcher + a throwaway per-process team root, so
/// every AppState construction site stays one line and parallel tests never
/// share a team share.
pub fn mock() -> Arc<TeamWebState> {
    let team_root =
        std::env::temp_dir().join(format!("opencoder-web-team-tests-{}", uuid::Uuid::new_v4()));
    Arc::new(TeamWebState::new(
        TeamRunConfig {
            team_root,
            max_turns: 8,
            max_sub_turns: 3,
        },
        Arc::new(opencoder_team::MockDispatcher::new()),
    ))
}

#[cfg(test)]
mod tests {
    use super::{data_dir_for, run_config_from};
    use opencoder_core::Config;
    use std::path::Path;

    /// Default (unset) team_root is rebased into the workdir's data dir,
    /// keeping the DB and the team share in one tree.
    #[test]
    fn unset_team_root_moves_into_the_workdir_data_dir() {
        let cfg = Config::default();
        let run = run_config_from(Path::new("/proj/x"), &cfg);
        assert_eq!(
            run.team_root,
            data_dir_for(Path::new("/proj/x")).join("team")
        );
        assert_eq!(run.max_turns, cfg.team_max_turns);
        assert_eq!(run.max_sub_turns, cfg.team_max_sub_turns);
    }

    /// An explicitly configured root survives verbatim.
    #[test]
    fn explicit_team_root_is_kept() {
        let mut cfg = Config::default();
        #[allow(clippy::field_reassign_with_default)] // clearer than struct-update here
        {
            cfg.team_root = Path::new("/nfs/share/teams").to_path_buf();
        }
        let run = run_config_from(Path::new("/proj/x"), &cfg);
        assert_eq!(run.team_root, Path::new("/nfs/share/teams"));
    }
}
