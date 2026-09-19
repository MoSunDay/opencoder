//! `dag.agent_sandbox = "runc"`: the agent step's WHOLE session (LLM loop +
//! tools + artifacts) runs inside a read-only OCI container.
//!
//! The host side stays thin and fail-closed, mirroring the wasm `run_runc`
//! posture: runc must exist, the knowledge root must be mountable, and the
//! API key must resolve — anything else errors the step instead of falling
//! back to the in-node host runner. The container launches the
//! `agent-step-runner` example (installed at `/usr/bin/agent-step-runner` by
//! `scripts/prepare-dag-rootfs.sh`) with `ArgvStyle::Direct`; the runner
//! owns `session.json`/`transcript.txt`/`output.json` inside the rw
//! `/workspace/context/<step>` bind, and the host only folds the terminal
//! result (exit 0 → `finish_from_output_json`; cancel/timeout/non-zero map
//! like the wasm branch).

use opencoder_dag::{StepKind, StepOutcome, StepSpec};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::agent::{build_prompt, create_session_meta, step_agent_name};
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
        &session_id,
    );
    info!(run_id = %ctx.run_id, step = %ctx.step.name, %session_id, "dag agent step executing (runc sandbox)");

    // Inputs the runner reads inside the container: the shared context
    // object plus the step prompt (the exact host-path prompt text).
    if let Err(e) = wasm::write_context_json(ctx) {
        return wasm::error_result(format!("cannot write context.json: {e}"));
    }
    let step_dir =
        match opencoder_dag::artifacts::step_dir(&ctx.workflow_root, &ctx.run_id, &ctx.step.name) {
            Ok(dir) => dir,
            Err(e) => return wasm::error_result(format!("illegal step path: {e}")),
        };
    if let Err(e) = std::fs::write(step_dir.join("prompt.txt"), build_prompt(ctx)) {
        return wasm::error_result(format!("cannot write prompt.txt: {e}"));
    }

    // LLM allowlist: the container has no config file, so the host injects
    // its own resolved endpoint (fail-closed on a missing API key).
    let api_key = match deps.config.api_key() {
        Ok(key) => key,
        Err(e) => return wasm::error_result(format!("agent sandbox needs an LLM API key: {e:#}")),
    };
    let mut env = wasm::step_env(ctx);
    env.push(("OPENCODER_MODEL".into(), deps.config.model_id().to_string()));
    env.push((
        "OPENAI_BASE_URL".into(),
        deps.config.base_url_for(deps.config.provider_id()),
    ));
    env.push(("OPENAI_API_KEY".into(), api_key));
    env.push(("OPENCODER_STEP_SESSION_ID".into(), session_id.clone()));
    env.push(("OPENCODER_STEP_AGENT".into(), step_agent_name(&ctx.step)));
    env.push((
        "OPENCODER_STEP_PROMPT".into(),
        format!("{}/{}/prompt.txt", CONTEXT_MOUNT, ctx.step.name),
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
        step_slug: ctx.step.name.clone(),
        command: vec![RUNNER_ARGV.into()],
        env,
        timeout_hint: ctx.step.timeout_secs,
        knowledge,
        argv: crate::sandbox::oci::ArgvStyle::Direct,
    };
    // Bundles live next to the run root, never inside it (the run root is
    // the guest-visible `/workspace/context` bind).
    let bundle_dir = ctx
        .workflow_root
        .join("bundles")
        .join(&ctx.run_id)
        .join(&ctx.step.name);
    let prepared =
        tokio::task::spawn_blocking(move || crate::sandbox::oci::write_bundle(&bundle_dir, &spec))
            .await;
    let bundle_dir = match prepared {
        Ok(Ok(dir)) => dir,
        Ok(Err(err)) => return wasm::error_result(format!("cannot build oci bundle: {err:#}")),
        Err(err) => return wasm::error_result(format!("oci bundle preparation failed: {err}")),
    };

    // run_step owns the timeout (it kills + reaps the container); every
    // stdout/stderr byte is mirrored into the node store as it arrives.
    let container_id = format!("{}-{}", ctx.run_id, ctx.step.name);
    let output = StepOutputLog::new(deps.store.clone(), &ctx.run_id, &ctx.step.name);
    let streamed = crate::sandbox::runc::run_step_streamed(
        &bundle_dir,
        &container_id,
        ctx.step.timeout_secs,
        cancel.clone(),
        Some(output.clone()),
    )
    .await;
    output.close().await;

    let mut result = match streamed {
        Ok((0, stdout)) => {
            let mut result = wasm::finish_from_output_json(&step_dir, stdout);
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
    // Successful step: persist the declared how.md append with the host
    // path's warn-only posture (a pool-write failure never flips Done).
    if result.outcome == StepOutcome::Done {
        if let Some(delta) = how_append_declared(&ctx.step).filter(|d| !d.trim().is_empty()) {
            match super::how_append::append_to_how_md(&step_agent_name(&ctx.step), delta) {
                Ok(version) => info!(
                    run_id = %ctx.run_id, step = %ctx.step.name, version,
                    "how_append persisted to agent prompt pool"
                ),
                Err(e) => warn!(
                    run_id = %ctx.run_id, step = %ctx.step.name, error = %e,
                    "how_append persistence failed (step outcome unchanged)"
                ),
            }
        }
    }
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
