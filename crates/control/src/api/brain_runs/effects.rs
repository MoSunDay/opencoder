//! Durable node outbox delivery. The source keeps replaying until the root
//! commits the receipt, then an acknowledgement is sent back to the source.
use crate::AppState;
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::*;
use serde_json::Value;
use std::sync::Arc;

pub async fn deliver(
    state: Arc<AppState>,
    node: String,
    execution: ExecutionRef,
    action: String,
    input: Value,
) -> Result<()> {
    let index = state
        .fleet
        .index(&execution.id)
        .await?
        .context("source execution is not registered")?;
    ensure!(
        index.node_id == node && index.kind == execution.kind,
        "outbox source ownership mismatch"
    );
    super::v4::delivery::deliver(&state, &node, &execution, &action, input).await
}
