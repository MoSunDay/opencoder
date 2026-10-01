use crate::Worker;
use anyhow::Result;
use opencoder_core::{harness::Harness, Config};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

/// Replaying this step completes interrupted initialization without resetting
/// a started Codex thread or its pinned Server settings.
pub(super) async fn ensure(
    worker: &Worker,
    id: &str,
    agent: &str,
    input: &Value,
    config: &Config,
    how_append: Option<&str>,
    config_home: Option<&Path>,
) -> Result<()> {
    let selection = input
        .get("harness")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()?;
    let envs = launch_environment(input, how_append, config_home)?;
    opencoder_core::agent::scope::with_root(
        config.agent.agents_dir.clone(),
        opencoder_session::harness::initialize(
            worker.inner.state.store.as_ref(),
            id,
            agent,
            selection,
            envs,
        ),
    )
    .await?;
    let mut runtime = worker
        .inner
        .state
        .store
        .harness_runtime(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("session harness initialization was not persisted"))?;
    if runtime.harness == Harness::Codex {
        opencoder_core::harness::pin_agent_settings(&mut runtime, config, agent)
            .map_err(anyhow::Error::msg)?;
        if runtime.codex.is_none()
            && runtime.thread_id.is_none()
            && runtime.last_input_id.is_none()
            && runtime.model.is_none()
        {
            runtime.model = input["model"].as_str().map(str::to_owned);
        }
    }
    if let Some(literal) = input["literal_mentions"].as_bool() {
        runtime.literal_mentions = literal;
    }
    worker
        .inner
        .state
        .store
        .set_harness_runtime(id, &runtime)
        .await
}

fn launch_environment(
    input: &Value,
    how_append: Option<&str>,
    config_home: Option<&Path>,
) -> Result<BTreeMap<String, String>> {
    let mut envs: BTreeMap<String, String> = input
        .get("envs")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()?
        .unwrap_or_default();
    if let Some(text) = how_append {
        envs.extend(opencoder_dag_runtime::exec::how_append::env_pairs(Some(
            text,
        )));
    }
    // Frozen operator HOME wins over user and managed environment values.
    if let Some(home) = config_home {
        envs.extend(crate::operations::operator_env::env_pairs(home));
    }
    Ok(envs)
}
