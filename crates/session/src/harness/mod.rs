//! Harness selection at the shared session seam; frontend-independent.
mod client;
pub mod codex;
pub mod resources;
pub mod title;

use crate::SessionState;
use anyhow::Result;
pub use client::configured_client;
use opencoder_core::harness::{Harness, HarnessRuntime};
use opencoder_store::Store;

pub async fn save(session: &SessionState) -> Result<()> {
    if let Some(store) = &session.store {
        store
            .set_harness_runtime(&session.id, &session.harness)
            .await?;
    }
    Ok(())
}

pub async fn prepare(session: &mut SessionState) -> Result<()> {
    opencoder_core::harness::pin_agent_settings(
        &mut session.harness,
        &session.config,
        &session.agent.name,
    )
    .map_err(anyhow::Error::msg)?;
    resources::prepare(session)?;
    if session.session_created
        && (session.harness.harness == Harness::Codex
            || session.harness.resource_root.is_some()
            || !session.harness.envs.is_empty())
    {
        save(session).await?;
    }
    Ok(())
}

/// Called before a fresh session accepts its first input, including web sessions
/// whose session row exists before a SessionState is constructed.
pub async fn initialize(
    store: &dyn Store,
    id: &str,
    agent: &str,
    selection: Option<Harness>,
    envs: std::collections::BTreeMap<String, String>,
) -> Result<()> {
    for (key, value) in &envs {
        opencoder_core::harness::validate_env(key, value).map_err(anyhow::Error::msg)?;
    }
    if let Some(existing) = store.harness_runtime(id).await? {
        anyhow::ensure!(
            selection.is_none_or(|s| s == existing.harness),
            "harness is fixed when the session starts"
        );
        anyhow::ensure!(
            envs.is_empty() || envs == existing.envs,
            "environment is fixed when the session starts"
        );
        return Ok(());
    }
    let legacy = store.last_message_seq(id).await? > 0;
    anyhow::ensure!(
        !legacy || selection.is_none_or(|h| h == Harness::Opencoder),
        "harness is fixed when the session starts"
    );
    let runtime = HarnessRuntime {
        harness: if legacy {
            Harness::Opencoder
        } else {
            selection.unwrap_or_else(|| opencoder_core::harness::agent_harness(agent))
        },
        envs,
        ..Default::default()
    };
    store.set_harness_runtime(id, &runtime).await
}
