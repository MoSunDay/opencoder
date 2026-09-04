//! Capability profiling: interview every member once and store its
//! self-described capabilities. Members whose interview fails keep their
//! existing capabilities — profiling is best-effort.

use std::sync::Arc;

use anyhow::Result;

use crate::config::TeamRunConfig;
use crate::decide::{ask_json, validate_profile};
use crate::dispatcher::TeamDispatcher;
use crate::fs_store;
use crate::prompts;
use crate::types::TeamMeta;
use opencoder_core::message::now_ms;

/// Interview every member once, store its self-described capabilities.
/// Members whose interview fails keep their existing capabilities.
pub async fn profile_team(
    dispatcher: Arc<dyn TeamDispatcher>,
    cfg: &TeamRunConfig,
    team_name: &str,
) -> Result<TeamMeta> {
    let mut team = fs_store::load_team(&cfg.team_root, team_name)?;
    let prompt = prompts::profile_prompt();
    for member in team.members.iter_mut() {
        let node_id = member.node_id.clone();
        match ask_json(
            dispatcher.as_ref(),
            None,
            &node_id,
            &prompt,
            validate_profile,
        )
        .await
        {
            Ok(decision) => {
                member.capabilities = decision
                    .capabilities
                    .into_iter()
                    .map(|c| c.trim().to_string())
                    .collect();
                member.profiled_at = Some(now_ms());
            }
            Err(error) => {
                tracing::warn!(node = %node_id, error = %format!("{error:#}"), "member profile failed")
            }
        }
    }
    team.updated_at = now_ms();
    fs_store::save_team(&cfg.team_root, &team)?;
    Ok(team)
}
