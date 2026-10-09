//! Sandbox Agent rounds drain the native session store in the existing writable
//! session directory. The host admits durable inputs, relays events/messages,
//! and keeps the same execution, capacity slot and pinned resources. All model
//! calls and tools stay inside runc. Missing runtime prerequisites fail closed.

use crate::{journal::Record, Worker};
use anyhow::{bail, Result};
use opencoder_agents::resources::how_append;
use opencoder_core::agent::{builtin_agents, read_agent_meta, scope, RunMode};
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use opencoder_core::Config;
use opencoder_dag_runtime::sandbox::oci::{write_bundle, BundleSpec};
use opencoder_dag_runtime::sandbox::runc::{run_step_streamed, runc_available};
use opencoder_session::handoff;
use opencoder_store::Store;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

use self::events::drain_events;
use super::agent_how::{agent_result, default_title, transcript_tail, OUTPUT_TAIL_BYTES};

/// Container argv of the session runner (rootfs-installed, Direct style).
const RUNNER_ARGV: &str = "/usr/bin/agent-session-runner";
/// Bounded tail for container exit diagnostics.
const ERROR_TAIL_BYTES: usize = 2048;
/// Event-tail polling cadence while the container runs.
const TAIL_POLL: std::time::Duration = std::time::Duration::from_millis(300);

/// Does the agent card under the pool `root` request the runc sandbox?
/// Builtin agents always resolve to the host registry, so they stay false;
/// a missing or corrupt card must never flip a session into the sandbox.
pub(crate) fn session_uses_sandbox(root: &Path, agent: &str) -> bool {
    if builtin_agents().iter().any(|builtin| builtin.name == agent) {
        return false;
    }
    scope::with_root_sync(Some(root.to_path_buf()), || {
        read_agent_meta(agent).is_some_and(|meta| meta.run_mode == RunMode::Agent)
    })
}

/// Whether `record`'s prompt must be intercepted and executed as a sandbox
/// round: an agent-kind execution whose pool (`scope` — the execution
/// resources root, or a prepared config's `agents_dir`) pins a
/// `run_mode: agent` card for its target. `None` scope (no snapshot on
/// disk) keeps the native path.
pub(crate) fn sandbox_session(record: &Record, scope: Option<&Path>) -> bool {
    scope.is_some_and(|root| {
        let request = &record.assignment.request;
        request.kind == ExecutionKind::Agent
            && session_uses_sandbox(root, request.target.as_deref().unwrap_or("act"))
    })
}

/// Fail-closed admission checks for a sandbox agent session, mirroring
/// `dag_preflight`: runc present, a real `<workflow root>/rootfs` tree and
/// a resolvable LLM key. Called from `create::prepare` (accept time) and
/// again by [`run_round`] (belt and braces).
pub(crate) fn preflight(worker: &Worker, config: &Config, legacy: bool) -> Result<()> {
    if !runc_available() {
        bail!("runc executable unavailable for requested agent sandbox");
    }
    let rootfs = sandbox_workflow_root(worker, legacy)?.join("rootfs");
    if !std::fs::symlink_metadata(&rootfs).is_ok_and(|meta| meta.is_dir()) {
        bail!(
            "runc rootfs unavailable at {}; prepare a real directory before creating the execution",
            rootfs.display()
        );
    }
    config.api_key()?;
    Ok(())
}

/// The workflow root owning sandbox run roots, the shared rootfs and the
/// bundle tree: the DAG kind root (new layout) or the legacy workflow root
/// (both are cleaned of owned containers on node start).
fn sandbox_workflow_root(worker: &Worker, legacy: bool) -> Result<PathBuf> {
    if legacy {
        worker.inner.layout.checked_legacy_workflow_root()
    } else {
        worker.inner.layout.checked_kind_root(ExecutionKind::Dag)
    }
}

/// Execute one sandbox turn — the whole `kind=agent` workload round for a
/// `run_mode: agent` session (see the module docs for the contract).
pub(super) async fn run_round(
    worker: &Worker,
    record: &Record,
    config: Config,
    cancel: CancellationToken,
    how_append: Option<String>,
) -> Result<(ExecutionStatus, Value)> {
    let assignment = &record.assignment;
    let id = assignment.index.id.clone();
    let agent = assignment
        .request
        .target
        .clone()
        .unwrap_or_else(|| "act".into());
    let input = &assignment.request.input;
    // Admission already ran the fail-closed checks; repeat them so a
    // degraded node (runc removed, rootfs gone, key rotated out) errors
    // the round instead of silently falling back to a host turn.
    let legacy = worker.inner.journal.lock().await.uses_legacy(&id);
    preflight(worker, &config, legacy)?;
    // The session row stays in the host store so consoles attach exactly
    // like a host session (SSE relay, messages, events).
    super::agent::create_session(
        worker,
        &id,
        &agent,
        input["model"].as_str().map(str::to_owned),
        assignment.index.created_at,
        &crate::brain::workdir::node_workdir(worker),
        super::agent::SessionLabels {
            title: default_title(assignment.request.kind, input["title"].as_str()),
            kind: Some(assignment.request.kind.prefix().to_string()),
        },
    )
    .await?;
    let runtime_store = inputs::open(worker, record, legacy).await?;
    runtime_store.recover_orphan_inputs(&id).await?;
    if !inputs::pending(runtime_store.as_ref(), &id).await? {
        // Monitor-only relaunch (no new turn).
        return Ok((ExecutionStatus::Idle, json!({"session_id": id})));
    }
    let workflow_root = sandbox_workflow_root(worker, legacy)?;
    let run_root = workflow_root.join(&id);
    if std::fs::symlink_metadata(&run_root).is_ok_and(|meta| !meta.is_dir()) {
        bail!(
            "sandbox run root is not a directory: {}",
            run_root.display()
        );
    }
    let session_dir = run_root.join("session");
    std::fs::create_dir_all(&session_dir)?;
    // Continuations keep container-owned harness state and resource snapshots.
    let mut runtime = match runtime_store.harness_runtime(&id).await? {
        Some(runtime) => runtime,
        None => worker
            .inner
            .state
            .store
            .harness_runtime(&id)
            .await?
            .unwrap_or_default(),
    };
    runtime.literal_mentions = input["literal_mentions"]
        .as_bool()
        .unwrap_or(runtime.literal_mentions);
    worker
        .inner
        .state
        .store
        .set_harness_runtime(&id, &runtime)
        .await?;
    runtime_store.set_harness_runtime(&id, &runtime).await?;
    std::fs::write(session_dir.join("events.ndjson"), b"")?;
    // The runner resolves its LLM endpoint from the injected env (the
    // container carries no config); `api_key` fails closed.
    let mut env = vec![
        (
            "OPENCODER_STEP_DIR".into(),
            "/workspace/context/session".into(),
        ),
        ("OPENCODER_STEP_SESSION_ID".into(), id.clone()),
        ("OPENCODER_STEP_AGENT".into(), agent.clone()),
        ("OPENCODER_AGENTS_DIR".into(), "/workspace/agent".into()),
        ("OPENCODER_MODEL".into(), config.model_id().to_string()),
        (
            "OPENAI_BASE_URL".into(),
            config.base_url_for(config.provider_id()),
        ),
        ("OPENAI_API_KEY".into(), config.api_key()?),
        // The agents pool is bound read-only; git must not need locks.
        ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
    ];
    env.extend(how_append::env_pairs(how_append.as_deref()));
    let spec = BundleSpec {
        run_root,
        step_slug: "agent-session".into(),
        command: vec![RUNNER_ARGV.into()],
        env,
        timeout_hint: None,
        knowledge: None,
        agents: config.agent.agents_dir.clone(),
    };
    // Bundles live outside the run root (which the guest sees as
    // /workspace/context) and are rebuilt per turn.
    let bundle_dir = workflow_root
        .join("bundles")
        .join("agent-sessions")
        .join(&id);
    if bundle_dir.exists() {
        let _ = std::fs::remove_dir_all(&bundle_dir);
    }
    let bundle_dir =
        match tokio::task::spawn_blocking(move || write_bundle(&bundle_dir, &spec)).await {
            Ok(Ok(dir)) => dir,
            Ok(Err(error)) => bail!("cannot build oci bundle: {error:#}"),
            Err(error) => bail!("oci bundle preparation failed: {error}"),
        };
    // Session ids may contain characters runc container ids must not; the
    // ULID also keeps retry rounds from colliding with a half-dead
    // predecessor.
    let container_id = format!("ag-{}", ulid::Ulid::new());
    let events_path = session_dir.join("events.ndjson");
    let container = run_step_streamed(&bundle_dir, &container_id, None, cancel.clone(), None);
    tokio::pin!(container);
    let store = worker.inner.state.store.clone();
    let mut offset = 0u64;
    let mut error_event: Option<String> = None;
    let outcome = loop {
        tokio::select! {
            result = &mut container => {
                inputs::sync_messages(worker, runtime_store.as_ref(), &id).await?;
                // Final drain passes until no new complete line appears.
                loop {
                    let next =
                        drain_events(&events_path, offset, store.as_ref(), &id, &mut error_event)
                            .await?;
                    if next == offset {
                        break;
                    }
                    offset = next;
                }
                break result;
            }
            _ = tokio::time::sleep(TAIL_POLL) => {
                inputs::sync_messages(worker, runtime_store.as_ref(), &id).await?;
                offset =
                    drain_events(&events_path, offset, store.as_ref(), &id, &mut error_event)
                        .await?;
            }
        }
    };
    match outcome {
        Ok((0, _)) => {
            if let Some(error) = error_event {
                bail!("agent execution failed: {error}");
            }
        }
        Ok((code, text)) => bail!(
            "runc agent session exited with {code}:\n{}",
            transcript_tail(&text, ERROR_TAIL_BYTES)
        ),
        Err(_) if cancel.is_cancelled() => {
            return Ok((ExecutionStatus::Cancelled, json!({"session_id": id})));
        }
        Err(error) => bail!("runc agent session failed: {error:#}"),
    }
    inputs::sync_messages(worker, runtime_store.as_ref(), &id).await?;
    let status = if cancel.is_cancelled() {
        ExecutionStatus::Cancelled
    } else {
        ExecutionStatus::Idle
    };
    if status != ExecutionStatus::Idle {
        return Ok((status, json!({"session_id": id})));
    }
    // The native path's success surface, byte for byte (the kind is always
    // Agent here): warn-only how_append persistence plus the bounded
    // output contract from the store projection.
    if let Some(delta) = how_append.as_deref().filter(|d| !d.trim().is_empty()) {
        match how_append::append_to_how_md(&agent, delta) {
            Ok(version) => {
                tracing::info!(session = %id, agent = %agent, version, "how_append persisted")
            }
            Err(error) => tracing::warn!(
                session = %id, agent = %agent, %error,
                "how_append persistence failed (execution outcome unchanged)"
            ),
        }
    }
    let messages = worker.inner.state.store.load_messages(&id).await?;
    let text = handoff::last_assistant_text(&messages).unwrap_or_default();
    let text = transcript_tail(&text, OUTPUT_TAIL_BYTES);
    let output_json = opencoder_session::harness::output::extract_output_json_from(&text);
    Ok((status, agent_result(&id, &text, output_json)))
}

mod events;
pub(crate) mod inputs;

#[cfg(test)]
mod tests;
