//! Read historical schema 4 projections without admitting or rewriting them.
use crate::Worker;
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
pub async fn read(
    worker: &Worker,
    reference: &ExecutionRef,
    action: &str,
    input: Value,
) -> Result<RpcReply> {
    ensure!(
        reference.kind == ExecutionKind::Brain,
        "expected brain root"
    );
    if !matches!(action, "snapshot" | "events") {
        return Ok(RpcReply::error(
            409,
            "historical schema 4 runs are read-only; convert the plan to schema 5",
        ));
    }
    let mut snapshot = worker
        .inner
        .state
        .store
        .brain_layered(&reference.id)
        .await?
        .context("historical layered projection missing")?;
    snapshot.schema_version = 4;
    if action == "snapshot" {
        return Ok(RpcReply::ok(json!(snapshot)));
    }
    let limit = input["limit"].as_u64().unwrap_or(100).clamp(1, 500) as u32;
    let events = worker
        .inner
        .state
        .store
        .brain_layered_events(&reference.id, input["after"].as_u64().unwrap_or(0), limit)
        .await?;
    Ok(RpcReply::ok(
        json!({"finished":snapshot.run.phase.terminal(),"more":events.len()==limit as usize,
        "next_seq":events.last().map(|e| e.seq),"events":events}),
    ))
}
