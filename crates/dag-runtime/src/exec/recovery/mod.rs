//! Same-session continuation for explicitly operator-managed steps.
use super::{ExecDeps, StepCtx};
use anyhow::{ensure, Result};
use opencoder_dag::RecoveryPolicy;
use std::sync::Arc;

pub(crate) async fn session_id(ctx: &StepCtx, deps: &ExecDeps) -> Result<String> {
    if ctx.step.recovery_policy == RecoveryPolicy::Operator {
        let path = ctx.dir().map_err(anyhow::Error::msg)?.join("session.json");
        if path.exists() {
            let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
            if let Some(id) = value["session_id"].as_str() {
                let meta = deps
                    .store
                    .get_session(id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("original step session unavailable"))?;
                ensure!(
                    meta.kind.as_deref() == Some("dag")
                        && meta.title.as_deref()
                            == Some(&format!("dag/{}/{}", ctx.run_id, ctx.step.name)),
                    "original step session ownership mismatch"
                );
                return Ok(id.into());
            }
        }
    }
    super::agent::create_session_meta(deps, &ctx.step, &ctx.run_id).await
}

/// The guest store retains messages and input-consumption checkpoints. A resume
/// adds a continuation instruction rather than submitting the original prompt.
pub async fn restore(session: &mut opencoder_session::SessionState) -> Result<bool> {
    use opencoder_store::Store;
    let store =
        Arc::new(opencoder_store::TursoStore::open(session.working_dir.join("runtime.db")).await?);
    if store.get_session(&session.id).await?.is_some() {
        let passthrough = std::mem::take(&mut session.env_passthrough);
        let cancel = session.cancel.clone();
        let mut recovered = opencoder_session::resume(
            store,
            &session.id,
            session.config.clone(),
            session.client.clone(),
            session.working_dir.clone(),
        )
        .await?;
        let has_input = !recovered.messages.is_empty();
        recovered.env_passthrough = passthrough;
        recovered.cancel = cancel;
        *session = recovered;
        Ok(has_input)
    } else {
        session.store = Some(store);
        Ok(false)
    }
}

pub const CONTINUE_PROMPT:&str="Continue the interrupted step in this existing session. Inspect the durable state and prior tool results before acting. Do not repeat an operation whose delivery or completion is unconfirmed; query its original identity first. Preserve completed work and produce the original required result.";

pub(crate) async fn completed(
    ctx: &StepCtx,
    deps: &ExecDeps,
    id: &str,
) -> Result<Option<super::StepResult>> {
    if ctx.step.recovery_policy != RecoveryPolicy::Operator {
        return Ok(None);
    }
    let workspace = super::native::io::run_root(ctx).join("workspace");
    let relative = std::path::PathBuf::from(ctx.relative_dir());
    let raw = match super::native::files::read_bounded(
        &workspace,
        &relative.join("session.json"),
        1024 * 1024,
    ) {
        Ok(raw) => raw,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    let receipt: serde_json::Value = serde_json::from_slice(&raw)?;
    ensure!(
        receipt["session_id"] == id,
        "guest session identity changed"
    );
    if receipt["status"] != "done" {
        return Ok(None);
    }
    super::native::io::archive(
        ctx,
        &[
            "session.json",
            "transcript.txt",
            "output.json",
            "artifacts.json",
            "messages.json",
        ],
    )?;
    ensure!(
        super::runc_events::import_messages(
            &workspace,
            &relative.join("messages.json"),
            deps.store.as_ref(),
            id
        )
        .await?,
        "completed session has no messages receipt"
    );
    let text = super::native::files::read_bounded(
        &workspace,
        &relative.join("transcript.txt"),
        64 * 1024,
    )?;
    let mut result = super::native::io::finish_from_output_json(
        &ctx.dir().map_err(anyhow::Error::msg)?,
        String::from_utf8(text)?,
    );
    result.session_id = Some(id.into());
    Ok(Some(result))
}
