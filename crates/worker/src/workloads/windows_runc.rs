use opencoder_core::{
    agent::{read_agent_meta, scope, RunMode},
    fleet::{ExecutionKind, ExecutionStatus},
    Config,
};
use serde_json::Value;
use std::path::Path;
use tokio_util::sync::CancellationToken;
pub(crate) fn session_uses_sandbox(root: &Path, agent: &str) -> bool {
    scope::with_root_sync(Some(root.to_path_buf()), || {
        read_agent_meta(agent).is_some_and(|meta| meta.run_mode == RunMode::Agent)
    })
}
pub(crate) fn sandbox_session(record: &crate::journal::Record, root: Option<&Path>) -> bool {
    record.assignment.request.kind == ExecutionKind::Agent
        && root.is_some_and(|root| {
            session_uses_sandbox(
                root,
                record.assignment.request.target.as_deref().unwrap_or("act"),
            )
        })
}
pub(crate) fn preflight(
    _worker: &crate::Worker,
    _config: &Config,
    _legacy: bool,
) -> anyhow::Result<()> {
    anyhow::bail!("agent sandbox execution requires Linux")
}
pub(super) async fn run_round(
    _worker: &crate::Worker,
    _record: &crate::journal::Record,
    _config: Config,
    _cancel: CancellationToken,
    _how_append: Option<String>,
) -> anyhow::Result<(ExecutionStatus, Value)> {
    anyhow::bail!("agent sandbox execution requires Linux")
}
