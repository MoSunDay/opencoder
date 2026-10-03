use opencoder_core::{
    brain::layered::{LayeredContext, LayeredDecision},
    Config,
};
use tokio_util::sync::CancellationToken;
pub async fn layered(
    _worker: &crate::Worker,
    _config: &Config,
    _context: &LayeredContext,
    _cancel: CancellationToken,
) -> anyhow::Result<LayeredDecision> {
    anyhow::bail!("Brain container execution requires Linux")
}
