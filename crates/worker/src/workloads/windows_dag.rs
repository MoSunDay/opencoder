use opencoder_core::{fleet::ExecutionStatus, Config};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
pub(super) async fn run(
    _worker: &crate::Worker,
    _record: &crate::journal::Record,
    _config: Config,
    _cancel: CancellationToken,
    _resume: bool,
) -> anyhow::Result<(ExecutionStatus, Value)> {
    anyhow::bail!("DAG execution requires Linux")
}
