//! `dag.agent_sandbox = "runc"`: the agent step's WHOLE session (LLM loop +
//! tools + artifacts) runs inside a read-only OCI container.
//!
//! The host side stays thin and fail-closed, mirroring the wasm `run_runc`
//! posture: runc must exist, the knowledge root must be mountable, and the
//! selected harness credentials must resolve — anything else errors the step instead of falling
//! back to the in-node host runner. The container launches the
//! `agent-step-runner` example (installed at `/usr/bin/agent-step-runner` by
//! `scripts/prepare-dag-rootfs.sh`) with `ArgvStyle::Direct`; the runner
//! owns `session.json`/`transcript.txt`/`output.json` inside the rw
//! `/workspace/context/<step>` bind, and the host only folds the terminal
//! result (exit 0 → `finish_from_output_json`; cancel/timeout/non-zero map
//! like the wasm branch).

use anyhow::Context;
use opencoder_dag::{StepKind, StepOutcome, StepSpec};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::agent::{build_prompt_with_knowledge, create_session_meta, step_agent_name};
use super::wasm::{self, CONTEXT_MOUNT};
use super::{ExecDeps, StepCtx, StepResult};
use crate::step_log::StepOutputLog;

/// Bounded tail for container stderr/exit diagnostics in `error`.
const ERROR_TAIL_BYTES: usize = 2048;

/// Container argv of the step runner (rootfs-installed, `ArgvStyle::Direct`).
const RUNNER_ARGV: &str = "/usr/bin/agent-step-runner";

/// The runc agent-step path (see the module docs). Every failure is an
/// Error outcome — never a silent host-runner fallback.
pub(crate) async fn execute_agent_step_runc(
    ctx: &StepCtx,
    deps: &ExecDeps,
    cancel: CancellationToken,
) -> StepResult {
    if !crate::sandbox::runc::runc_available() {
        return wasm::error_result(
            "runc executable unavailable for requested agent sandbox (fail-closed; \
             host session is the default)"
                .into(),
        );
    }
    // The session row + live pointer stay on the host store so a remote
    // console can attach exactly like the host path.
    let session_id = match create_session_meta(deps, &ctx.step, &ctx.run_id).await {
        Ok(id) => id,
        Err(e) => return wasm::error_result(format!("create session: {e:#}")),
    };
    crate::step_io::write_session_artifact(
        &ctx.workflow_root,
        &ctx.run_id,
        &ctx.step.name,
        ctx.instance,
        &session_id,
    );
    info!(run_id = %ctx.run_id, step = %ctx.step.name, %session_id, "dag agent step executing (runc sandbox)");

    // Inputs the runner reads inside the container: the shared context
    // object plus the step prompt (the exact host-path prompt text).
    if let Err(e) = wasm::write_context_json(ctx) {
        return wasm::error_result(format!("cannot write context.json: {e}"));
    }
    let step_dir = match ctx.dir() {
        Ok(dir) => dir,
        Err(e) => return wasm::error_result(format!("illegal step path: {e}")),
    };
    let knowledge_path = ctx
        .knowledge_root
        .as_ref()
        .map(|_| std::path::Path::new(super::KNOWLEDGE_MOUNT));
    if let Err(e) =
        std::fs::write(
            step_dir.join("prompt.txt"),
            super::private_files::prompt(
                build_prompt_with_knowledge(ctx, knowledge_path),
                deps.config.dag.execution_private_root.as_ref().map(|_| {
                    std::path::Path::new(opencoder_core::fleet::private_files::GUEST_ROOT)
                }),
            ),
        )
    {
        return wasm::error_result(format!("cannot write prompt.txt: {e}"));
    }

    let mut config = deps.config.clone();
    if let StepKind::Agent {
        model: Some(model), ..
    } = &ctx.step.kind
    {
        config.model = model.clone();
    }
    let mut codex = match crate::sandbox::codex::resolve(
        &config,
        &step_agent_name(&ctx.step),
        &ctx.workflow_root.join("rootfs"),
    ) {
        Ok(launch) => launch,
        Err(e) => return wasm::error_result(format!("Codex sandbox preflight: {e:#}")),
    };
    if let Some(launch) = &mut codex {
        if let StepKind::Agent {
            model: Some(model), ..
        } = &ctx.step.kind
        {
            launch.runtime.model = Some(model.clone());
        }
        if let Err(e) = deps
            .store
            .set_harness_runtime(&session_id, &launch.runtime)
            .await
        {
            return wasm::error_result(format!("persist Codex sandbox settings: {e:#}"));
        }
    }
    let mut env = wasm::step_env(ctx);
    if deps.config.agent.agents_dir.is_some() {
        env.push(("OPENCODER_AGENTS_DIR".into(), super::AGENTS_MOUNT.into()));
    }
    env.push(("OPENCODER_MODEL".into(), config.model_id().to_string()));
    if codex.is_none() {
        let api_key = match config.api_key() {
            Ok(key) => key,
            Err(e) => {
                return wasm::error_result(format!("agent sandbox needs an LLM API key: {e:#}"))
            }
        };
        env.push((
            "OPENAI_BASE_URL".into(),
            config.base_url_for(config.provider_id()),
        ));
        env.push(("OPENAI_API_KEY".into(), api_key));
    }
    env.push(("OPENCODER_STEP_SESSION_ID".into(), session_id.clone()));
    env.push(("OPENCODER_STEP_AGENT".into(), step_agent_name(&ctx.step)));
    env.push((
        "OPENCODER_STEP_PROMPT".into(),
        format!("{}/{}/prompt.txt", CONTEXT_MOUNT, ctx.relative_dir()),
    ));
    if let Some(pairs) = how_append_env(ctx) {
        env.extend(pairs);
    }
    // A read-only knowledge checkout must never refresh a git index.
    env.push(("GIT_OPTIONAL_LOCKS".into(), "0".into()));

    let knowledge = match wasm::knowledge_mount(ctx) {
        Ok(k) => k,
        Err(e) => return wasm::error_result(e),
    };
    let spec = crate::sandbox::oci::BundleSpec {
        run_root: ctx.workflow_root.join(&ctx.run_id),
        step_slug: ctx.relative_dir(),
        command: vec![RUNNER_ARGV.into()],
        env,
        timeout_hint: ctx.step.timeout_secs,
        knowledge,
        // Tools and skills resolve against the same read-only pinned pool.
        agents: deps.config.agent.agents_dir.clone(),
        argv: crate::sandbox::oci::ArgvStyle::Direct,
    };
    // Bundles live next to the run root, never inside it (the run root is
    // the guest-visible `/workspace/context` bind).
    let bundle_dir = ctx
        .workflow_root
        .join("bundles")
        .join(&ctx.run_id)
        .join(ctx.relative_dir());
    let launch = codex.clone();
    let prepared = tokio::task::spawn_blocking(move || match launch {
        Some(launch) => launch.write_bundle(&bundle_dir, &spec),
        None => crate::sandbox::oci::write_bundle(&bundle_dir, &spec),
    })
    .await;
    let bundle_dir = match prepared {
        Ok(Ok(dir)) => dir,
        Ok(Err(err)) => return wasm::error_result(format!("cannot build oci bundle: {err:#}")),
        Err(err) => return wasm::error_result(format!("oci bundle preparation failed: {err}")),
    };

    if let Some(root) = deps.config.dag.execution_private_root.as_deref() {
        if let Err(error) = super::private_files::bind(&bundle_dir, root) {
            return wasm::error_result(format!("private task mount failed: {error:#}"));
        }
    }

    // run_step owns the timeout (it kills + reaps the container); every
    // stdout/stderr byte is mirrored into the node store as it arrives.
    let container_id = format!("{}-{}", ctx.run_id, ctx.execution_key());
    let output = StepOutputLog::for_instance(
        deps.store.clone(),
        &ctx.run_id,
        &ctx.step.name,
        ctx.instance,
    );
    let container = crate::sandbox::runc::run_step_streamed(
        &bundle_dir,
        &container_id,
        ctx.step.timeout_secs,
        cancel.clone(),
        Some(output.clone()),
    );
    tokio::pin!(container);
    let mut offset = 0;
    let events_path = step_dir.join("events.ndjson");
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    let mut event_error = None;
    let streamed = loop {
        tokio::select! {
            result = &mut container => break result,
            _ = tick.tick() => {
                if let Err(error) = super::runc_events::drain(&events_path, &mut offset, deps.store.as_ref(), &session_id).await {
                    event_error = Some(error);
                    cancel.cancel();
                    break container.await;
                }
            }
        }
    };
    loop {
        let before = offset;
        if let Err(error) =
            super::runc_events::drain(&events_path, &mut offset, deps.store.as_ref(), &session_id)
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
    if let Some(mut launch) = codex {
        // Only the thread pointer returns from the guest; never import guest
        // environment/settings into the node's private harness configuration.
        let thread = (|| -> anyhow::Result<Option<String>> {
            let bytes = std::fs::read(step_dir.join("session.json"))
                .context("read Codex sandbox session receipt")?;
            let meta: serde_json::Value =
                serde_json::from_slice(&bytes).context("parse Codex sandbox session receipt")?;
            anyhow::ensure!(
                meta["session_id"] == session_id,
                "Codex sandbox session id mismatch"
            );
            let thread = meta["thread_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            anyhow::ensure!(
                !matches!(&streamed, Ok((0, _))) || thread.is_some(),
                "Codex sandbox completed without a thread receipt"
            );
            Ok(thread)
        })();
        match thread {
            Ok(thread) => launch.runtime.thread_id = thread,
            Err(error) => event_error = Some(error),
        }
        if let Err(error) = deps
            .store
            .set_harness_runtime(&session_id, &launch.runtime)
            .await
        {
            event_error = Some(error.context("persist Codex sandbox thread"));
        }
    }
    if let Some(error) = event_error {
        let mut result = wasm::error_result(format!("container event persistence: {error:#}"));
        result.session_id = Some(session_id);
        return result;
    }

    let mut result = match streamed {
        Ok((0, stdout)) => {
            let transcript = match std::fs::read_to_string(step_dir.join("transcript.txt")) {
                Ok(text) => text,
                Err(error) => {
                    return wasm::error_result(format!(
                        "container transcript: {error:#}; stdout: {stdout}"
                    ))
                }
            };
            let mut result = wasm::finish_from_output_json(&step_dir, transcript);
            result.session_id = Some(session_id.clone());
            result
        }
        Ok((code, text)) => {
            warn!(run_id = %ctx.run_id, step = %ctx.step.name, code,
                "runc agent step exited non-zero (session.json state written by the runner)");
            wasm::error_result(format!(
                "runc agent step exited with {code}:\n{}",
                wasm::tail(&text, ERROR_TAIL_BYTES)
            ))
        }
        Err(err) if cancel.is_cancelled() => StepResult {
            outcome: StepOutcome::Cancelled,
            ..wasm::error_result(err.to_string())
        },
        Err(err) => {
            warn!(run_id = %ctx.run_id, step = %ctx.step.name, error = %err,
                "runc agent step failed (session.json state written by the runner)");
            wasm::error_result(err.to_string())
        }
    };
    result.session_id = Some(session_id);

    result
}

/// The step's declared `how_append` payload, when any.
fn how_append_declared(step: &StepSpec) -> Option<&str> {
    match &step.kind {
        StepKind::Agent { how_append, .. } => how_append.as_deref(),
        _ => None,
    }
}

/// The `OPENCODER_HOW_APPEND` env pair for the container runner.
fn how_append_env(ctx: &StepCtx) -> Option<Vec<(String, String)>> {
    let pairs = super::how_append::env_pairs(how_append_declared(&ctx.step));
    (!pairs.is_empty()).then_some(pairs)
}
