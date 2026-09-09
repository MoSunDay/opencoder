//! Deterministic business workflows executed as registered foreground binaries.
pub mod artifacts;
pub mod configuration;
mod events;
mod process;

use super::{ExecDeps, StepCtx, StepResult};
use anyhow::{ensure, Context, Result};
use opencoder_core::{
    harness::{agent_settings, RunnerSettings},
    Config,
};
use opencoder_dag::{StepKind, StepOutcome};
use opencoder_session::SessionState;
use serde_json::{json, Value};
use std::{collections::BTreeMap, time::Duration};
use tokio_util::sync::CancellationToken;

pub fn validate(config: &Config, runner: &str, agent: &str) -> Result<()> {
    let registered = config
        .agent
        .runtime
        .runners
        .get(runner)
        .context("Runner registration unavailable")?;
    artifacts::validate_registration(&registered.settings)?;
    ensure!(
        opencoder_core::harness::agent_harness(agent) == opencoder_core::harness::Harness::Codex,
        "Runner agent must use Codex"
    );
    let settings = agent_settings(config, agent)
        .map_err(anyhow::Error::msg)?
        .context("Runner Codex settings unavailable")?;
    settings.validate().map_err(anyhow::Error::msg)?;
    opencoder_session::harness::codex::configured_binary(
        Some(settings),
        &settings.envs,
        &registered.settings.workdir,
    )?;
    Ok(())
}

pub async fn execute(ctx: &StepCtx, deps: &ExecDeps, cancel: CancellationToken) -> StepResult {
    let result = run(ctx, deps, cancel.clone()).await;
    match result {
        Ok(value) => value,
        Err(error) => StepResult {
            outcome: if cancel.is_cancelled() {
                StepOutcome::Cancelled
            } else {
                StepOutcome::Error
            },
            error: Some(format!("{error:#}")),
            output_text: String::new(),
            output_json: None,
            session_id: Some(ctx.run_id.clone()),
        },
    }
}

async fn run(ctx: &StepCtx, deps: &ExecDeps, cancel: CancellationToken) -> Result<StepResult> {
    let StepKind::Runner { runner, agent } = &ctx.step.kind else {
        anyhow::bail!("expected Runner step");
    };
    let dir = opencoder_dag::artifacts::step_dir(&ctx.workflow_root, &ctx.run_id, &ctx.step.name)
        .map_err(anyhow::Error::msg)?;
    let output = dir.join("artifacts");
    std::fs::create_dir_all(&output)?;
    let completion = dir.join("runner-completion.json");
    if completion.exists() {
        let saved: Value = serde_json::from_slice(&std::fs::read(completion)?)?;
        artifacts::verify(
            &output,
            &serde_json::from_value::<Vec<artifacts::Artifact>>(saved["artifacts"].clone())?,
            saved["result_file"]
                .as_str()
                .context("Runner receipt result missing")?,
        )?;
        return Ok(serde_json::from_value(saved["step_result"].clone())?);
    }
    let marker = dir.join("runner-started.json");
    ensure!(
        !marker.exists(),
        "Runner interrupted without a validated result; retry as a new business attempt"
    );
    let registered = deps
        .config
        .agent
        .runtime
        .runners
        .get(runner)
        .context("Runner registration unavailable")?;
    opencoder_core::agent::scope::with_root_sync(deps.config.agent.agents_dir.clone(), || {
        validate(&deps.config, runner, agent)
    })?;
    let resolved =
        opencoder_core::agent::scope::with_root_sync(deps.config.agent.agents_dir.clone(), || {
            opencoder_core::resolve_agent(agent)
        })
        .context("Runner agent unavailable")?;
    let mut session = SessionState::new(
        ctx.run_id.clone(),
        resolved,
        deps.config.clone(),
        deps.client.clone(),
        deps.workdir.clone(),
    );
    session = session
        .with_store(deps.store.clone())
        .mark_session_created();
    opencoder_core::agent::scope::with_root_sync(deps.config.agent.agents_dir.clone(), || {
        opencoder_session::harness::resources::prepare(&mut session)
    })?;
    let settings = session
        .harness
        .codex
        .as_ref()
        .context("Runner Codex profile missing")?;
    let metadata = configuration::snapshot(&deps.config, runner, agent);
    let profile = &metadata["profile"];
    let profile_revision = &metadata["profile_revision"];
    let input: Value = serde_json::from_slice(&std::fs::read(
        ctx.workflow_root.join(&ctx.run_id).join("input.json"),
    )?)?;
    let timeout = ctx.step.timeout_secs.unwrap_or(3600);
    ensure!(timeout > 0, "Runner timeout must be positive");
    let unit = process::unit(&registered.settings);
    let invocation = json!({"schema":"opencoder.runner.v1","execution_id":ctx.run_id,"step":ctx.step.name,
        "input":input,"context":ctx.context(),"output_dir":output,"timeout_secs":timeout,"unit":unit,
        "agent":{"name":agent,"prompt":session.agent.prompt,"resources":session.harness.resource_root},
        "codex":settings,"profile":profile,"profile_revision":profile_revision,"runner_revision":registered.revision});
    crate::checkpoint::write(&marker, &serde_json::to_vec(&metadata)?)?;
    let mut stream = events::Events {
        status_path: dir.join("runner-status.json"),
        decoders: BTreeMap::new(),
        result: None,
        prefix: format!("runner:{}:{}", ctx.step.name, ulid::Ulid::new()),
        step: ctx.step.name.clone(),
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    let mut process = tokio::time::timeout_at(
        deadline,
        process::Running::spawn(&registered.settings, unit, timeout, &invocation),
    )
    .await
    .context("Runner startup timed out")??;
    let result: Result<()> = async {
        loop {
            let line = tokio::select! {
                biased;
                _ = cancel.cancelled() => anyhow::bail!("Runner cancelled"),
                _ = tokio::time::sleep_until(deadline) => anyhow::bail!("Runner deadline exceeded"),
                line = process.lines.next_line() => line?,
            };
            let Some(line) = line else {
                break;
            };
            ensure!(
                line.len() <= 16 * 1024 * 1024,
                "Runner event exceeds 16 MiB"
            );
            stream
                .accept(
                    &mut session,
                    serde_json::from_str(&line).context("invalid Runner event")?,
                )
                .await?;
        }
        process.finish().await?;
        ensure!(
            stream.result.is_some(),
            "Runner exited without a result manifest"
        );
        Ok(())
    }
    .await;
    if let Err(error) = result {
        process.stop().await?;
        stream.interrupt(&mut session, "Runner interrupted").await?;
        return Err(redacted_error(
            error,
            &registered.settings,
            &invocation["codex"]["envs"],
        ));
    }
    let (result_file, manifest) = stream.result.unwrap();
    let value = artifacts::verify(&output, &manifest, &result_file)?;
    let output_json = json!({"result":value,"artifacts":manifest,"runner":runner,"runner_revision":registered.revision,
        "agent":agent,"profile":profile,"profile_revision":profile_revision});
    let step_result = StepResult {
        outcome: StepOutcome::Done,
        error: None,
        output_text: value["summary"]
            .as_str()
            .unwrap_or("Workflow completed")
            .into(),
        output_json: Some(output_json),
        session_id: Some(ctx.run_id.clone()),
    };
    crate::checkpoint::write(
        &completion,
        &serde_json::to_vec(
            &json!({"artifacts":manifest,"result_file":result_file,"step_result":step_result}),
        )?,
    )?;
    Ok(step_result)
}

fn redacted_error(
    error: anyhow::Error,
    settings: &RunnerSettings,
    codex_envs: &Value,
) -> anyhow::Error {
    let mut text = format!("{error:#}");
    for value in settings
        .envs
        .values()
        .map(String::as_str)
        .chain(
            codex_envs
                .as_object()
                .into_iter()
                .flat_map(|map| map.values())
                .filter_map(Value::as_str),
        )
        .filter(|value| !value.is_empty())
    {
        text = text.replace(value, "[redacted]");
    }
    anyhow::anyhow!(text)
}
