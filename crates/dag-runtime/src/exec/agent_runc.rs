use anyhow::{ensure, Context, Result};
use opencoder_dag::{StepKind, StepOutcome};
use tokio_util::sync::CancellationToken;

use super::agent::{build_prompt_with_knowledge, create_session_meta, step_agent_name};
use super::native::io;
use super::{ExecDeps, StepCtx, StepResult};
use crate::step_log::StepOutputLog;

pub(crate) async fn execute_agent_step_runc(
    ctx: &StepCtx,
    deps: &ExecDeps,
    cancel: CancellationToken,
) -> StepResult {
    let session_id = match create_session_meta(deps, &ctx.step, &ctx.run_id).await {
        Ok(id) => id,
        Err(error) => return io::error_result(format!("create session: {error:#}")),
    };
    crate::step_io::write_session_artifact(
        &ctx.workflow_root,
        &ctx.run_id,
        &ctx.step.name,
        ctx.instance,
        &session_id,
    );
    let mut result = match execute_session(ctx, deps, cancel.child_token(), &session_id).await {
        Ok(result) => result,
        Err(error) => StepResult {
            outcome: if cancel.is_cancelled() {
                StepOutcome::Cancelled
            } else {
                StepOutcome::Error
            },
            output_text: error
                .downcast_ref::<crate::sandbox::run::ProcessFailure>()
                .map(|failure| failure.output.clone())
                .unwrap_or_default(),
            ..io::error_result(format!("Agent container execution: {error:#}"))
        },
    };
    result.session_id = Some(session_id);
    result
}

async fn execute_session(
    ctx: &StepCtx,
    deps: &ExecDeps,
    cancel: CancellationToken,
    session_id: &str,
) -> Result<StepResult> {
    io::write_context_json(ctx)?;
    let meta = io::meta_dir(ctx).map_err(anyhow::Error::msg)?;
    let knowledge = ctx
        .knowledge_root
        .as_ref()
        .map(|_| std::path::Path::new(io::KNOWLEDGE_MOUNT));
    let prompt = build_prompt_with_knowledge(ctx, knowledge);
    let prompt = super::private_files::prompt(
        prompt,
        deps.config
            .dag
            .execution_private_root
            .as_ref()
            .map(|_| std::path::Path::new(opencoder_core::fleet::private_files::GUEST_ROOT)),
    );
    opencoder_core::atomic_write(&meta.join("prompt.txt"), prompt.as_bytes())?;
    let mut env = io::step_env(ctx);
    let mut config = deps.config.clone();
    if let StepKind::Agent {
        model: Some(model), ..
    } = &ctx.step.kind
    {
        config.model = model.clone();
    }
    let launch_path = io::run_root(ctx)
        .join("private/codex")
        .join(&ctx.step.name)
        .join("launch.json");
    let mut runtime: Option<opencoder_core::harness::HarnessRuntime> =
        match std::fs::read(&launch_path) {
            Ok(bytes) => Some(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
    if let Some(runtime) = &mut runtime {
        if let StepKind::Agent {
            model: Some(model), ..
        } = &ctx.step.kind
        {
            runtime.model = Some(model.clone());
        }
        deps.store.set_harness_runtime(session_id, runtime).await?;
        let name = format!("{}.json", ctx.execution_key());
        let path = launch_path
            .parent()
            .context("Codex launch parent missing")?
            .join(&name);
        opencoder_core::atomic_write(&path, &serde_json::to_vec(runtime)?)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        env.push((
            "OPENCODER_CODEX_LAUNCH".into(),
            format!("/run/opencoder-private/codex/{}/{name}", ctx.step.name),
        ));
    }
    let endpoint = if runtime.is_some() {
        None
    } else {
        Some(config.resolve_endpoint()?)
    };
    let launch = super::native::launch::guest_config(&config, endpoint);
    let private = io::run_root(ctx).join("private/native");
    std::fs::create_dir_all(&private)?;
    let name = format!("{}.json", ctx.execution_key());
    opencoder_core::atomic_write(&private.join(&name), &serde_json::to_vec(&launch)?)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(private.join(&name), std::fs::Permissions::from_mode(0o600))?;
    env.push((
        "OPENCODER_STEP_CONFIG".into(),
        format!("/run/opencoder-private/native/{name}"),
    ));
    if config.agent.agents_dir.is_some() {
        env.push(("OPENCODER_AGENTS_DIR".into(), io::AGENTS_MOUNT.into()));
    }
    env.extend([
        ("OPENCODER_MODEL".into(), config.model_id().to_string()),
        ("OPENCODER_STEP_SESSION_ID".into(), session_id.to_string()),
        ("OPENCODER_STEP_AGENT".into(), step_agent_name(&ctx.step)),
        (
            "OPENCODER_STEP_PROMPT".into(),
            format!("{}/prompt.txt", io::guest_meta(ctx)),
        ),
        ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
    ]);
    if let StepKind::Agent { how_append, .. } = &ctx.step.kind {
        env.extend(super::how_append::env_pairs(how_append.as_deref()));
    }
    let output = StepOutputLog::for_instance(
        deps.store.clone(),
        &ctx.run_id,
        &ctx.step.name,
        ctx.instance,
    );
    let root = io::run_root(ctx);
    let key = ctx.execution_key();
    let cwd = format!("{}/{}", io::CONTEXT_MOUNT, ctx.relative_dir());
    let process = crate::sandbox::run::StepProcess {
        key,
        argv: vec!["/usr/bin/agent-step-runner".into()],
        env,
        cwd,
        timeout_secs: ctx.step.timeout_secs,
    };
    let execution =
        crate::sandbox::run::execute(&root, process, cancel.clone(), Some(output.clone()));
    tokio::pin!(execution);
    let mut offset = 0;
    let events = std::path::Path::new(&ctx.relative_dir())
        .join("agent-events")
        .join(format!("{session_id}.ndjson"));
    let workspace = root.join("workspace");
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    let mut event_error = None;
    let streamed = loop {
        tokio::select! {
            result = &mut execution => break result,
            _ = tick.tick() => {
                if let Err(error) = super::runc_events::drain(&workspace, &events, &mut offset, deps.store.as_ref(), session_id, ctx.log.as_ref()).await {
                    event_error = Some(error);
                    cancel.cancel();
                    break execution.await;
                }
            }
        }
    };
    loop {
        let before = offset;
        if let Err(error) = super::runc_events::drain(
            &workspace,
            &events,
            &mut offset,
            deps.store.as_ref(),
            session_id,
            ctx.log.as_ref(),
        )
        .await
        {
            event_error = Some(error);
            break;
        }
        if before == offset {
            break;
        }
    }
    output.close().await;
    io::archive(
        ctx,
        &[
            "session.json",
            "transcript.txt",
            "output.json",
            "artifacts.json",
        ],
    )?;
    let messages_present = super::runc_events::import_messages(
        &workspace,
        &std::path::Path::new(&ctx.relative_dir()).join("messages.json"),
        deps.store.as_ref(),
        session_id,
    )
    .await?;
    if let Some(runtime) = &mut runtime {
        let bytes = super::native::files::read_bounded(
            &workspace,
            &std::path::Path::new(&ctx.relative_dir()).join("session.json"),
            crate::sandbox::output_limit::STRUCTURED_JSON_LIMIT_BYTES,
        )?;
        let receipt: serde_json::Value = serde_json::from_slice(&bytes)?;
        ensure!(
            receipt["session_id"] == session_id,
            "Codex session receipt mismatch"
        );
        runtime.thread_id = receipt["thread_id"]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        ensure!(
            !matches!(streamed, Ok((0, _))) || runtime.thread_id.is_some(),
            "Codex completed without thread receipt"
        );
        deps.store.set_harness_runtime(session_id, runtime).await?;
    }
    if let Some(error) = event_error {
        return Err(error.context("persist Agent events"));
    }
    let (code, text) = streamed?;
    if code != 0 {
        return Ok(StepResult {
            output_text: text.clone(),
            ..io::error_result(format!(
                "Agent exited with {code}: {}",
                io::tail(&text, 2048)
            ))
        });
    }
    ensure!(messages_present, "Agent completed without messages receipt");
    let transcript = super::native::files::read_bounded(
        &workspace,
        &std::path::Path::new(&ctx.relative_dir()).join("transcript.txt"),
        64 * 1024,
    )?;
    Ok(io::finish_from_output_json(
        &ctx.dir().map_err(anyhow::Error::msg)?,
        String::from_utf8(transcript)?,
    ))
}
