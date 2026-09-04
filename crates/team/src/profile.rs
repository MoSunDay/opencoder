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
///
/// Each successful interview is merged narrowly: the team is re-loaded fresh
/// from disk and only that member's `capabilities` + `profiled_at` are
/// patched before saving, so concurrent management edits (captain swap,
/// membership add/remove, other profile writes) are never rolled back.
pub async fn profile_team(
    dispatcher: Arc<dyn TeamDispatcher>,
    cfg: &TeamRunConfig,
    team_name: &str,
) -> Result<TeamMeta> {
    let team = fs_store::load_team(&cfg.team_root, team_name)?;
    let prompt = prompts::profile_prompt();
    // Snapshot of node ids up front: the member set may change under us.
    let node_ids: Vec<String> = team.members.iter().map(|m| m.node_id.clone()).collect();
    for node_id in node_ids {
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
                let mut fresh = fs_store::load_team(&cfg.team_root, team_name)?;
                let Some(member) = fresh.members.iter_mut().find(|m| m.node_id == node_id) else {
                    tracing::info!(node = %node_id, "member removed concurrently; dropping profile result");
                    continue;
                };
                member.capabilities = decision
                    .capabilities
                    .into_iter()
                    .map(|c| c.trim().to_string())
                    .collect();
                member.profiled_at = Some(now_ms());
                fresh.updated_at = now_ms();
                // Saving after each member also bounds crash loss to one interview.
                fs_store::save_team(&cfg.team_root, &fresh)?;
            }
            Err(error) => {
                tracing::warn!(node = %node_id, error = %format!("{error:#}"), "member profile failed")
            }
        }
    }
    fs_store::load_team(&cfg.team_root, team_name)
}
