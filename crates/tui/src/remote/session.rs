use anyhow::{Context, Result};
use opencoder_core::{
    harness::{HarnessRuntime, RemoteSession},
    Config,
};
use opencoder_llm::ChatStream;
use opencoder_session::SessionState;
use opencoder_store::{SessionMeta, Store};
use std::{path::Path, sync::Arc};

pub(crate) async fn create(
    remote: RemoteSession,
    config: Config,
    client: Arc<dyn ChatStream>,
    store: Arc<dyn Store>,
    workdir: &Path,
) -> Result<SessionState> {
    let id = format!(
        "{}-{}",
        remote.capability.kind.prefix(),
        opencoder_session::runner::new_id()
    );
    let now = opencoder_core::message::now_ms();
    store
        .create_session(&SessionMeta {
            id: id.clone(),
            kind: Some(format!("tui_remote_{}", remote.capability.kind.prefix())),
            title: Some(remote.capability.id.clone()),
            agent: Some(remote.capability.target.clone()),
            created_at: now,
            updated_at: now,
            ..Default::default()
        })
        .await?;
    let runtime = HarnessRuntime {
        remote: Some(remote),
        ..Default::default()
    };
    store.set_harness_runtime(&id, &runtime).await?;
    shell(&id, runtime, config, client, store, workdir)
}

pub(crate) async fn load(
    id: &str,
    config: Config,
    client: Arc<dyn ChatStream>,
    store: Arc<dyn Store>,
    workdir: &Path,
) -> Result<Option<SessionState>> {
    let runtime = store.harness_runtime(id).await?;
    let is_remote = store.get_session(id).await?.is_some_and(|meta| {
        meta.kind
            .as_deref()
            .is_some_and(|kind| kind.starts_with("tui_remote_"))
    });
    anyhow::ensure!(
        !is_remote
            || runtime
                .as_ref()
                .is_some_and(|runtime| runtime.remote.is_some()),
        "Remote task bookmark is incomplete; its Server binding is missing"
    );
    let Some(runtime) = runtime else {
        return Ok(None);
    };
    if runtime.remote.is_none() {
        return Ok(None);
    }
    anyhow::ensure!(
        config.opencoder_server.enabled,
        "This is a remote task; enable opencoder_server.enabled to resume it"
    );
    shell(id, runtime, config, client, store, workdir).map(Some)
}

fn shell(
    id: &str,
    runtime: HarnessRuntime,
    config: Config,
    client: Arc<dyn ChatStream>,
    store: Arc<dyn Store>,
    workdir: &Path,
) -> Result<SessionState> {
    let binding = runtime
        .remote
        .as_ref()
        .context("remote task binding missing")?;
    anyhow::ensure!(
        matches!(
            binding.capability.kind,
            opencoder_core::fleet::ExecutionKind::Agent
                | opencoder_core::fleet::ExecutionKind::Operator
        ),
        "Only Agent and Operator tasks can be resumed in TUI"
    );
    anyhow::ensure!(
        opencoder_core::fleet::valid_id(id)
            && id.starts_with(&format!("{}-", binding.capability.kind.prefix())),
        "Remote task identity does not match its capability"
    );
    // The local shell supplies UI handles only. Its worker never runs a local
    // agent, reads local credentials or resolves the server's card locally.
    let mut session = SessionState::new(
        id,
        opencoder_core::resolve_agent("act").context("builtin act")?,
        config,
        client,
        workdir.to_owned(),
    )
    .with_store(store)
    .mark_session_created();
    session.harness = runtime;
    session.harness.literal_mentions = true;
    Ok(session)
}
