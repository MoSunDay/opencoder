//! Reads of the node-owned layered projection and capability outputs.
use crate::{api::brain_runs::runs, AppState};
use anyhow::Result;
use opencoder_core::{brain::layered::*, fleet::*};
use serde_json::{json, Value};
use std::sync::Arc;

const EVENT_PAGE: u32 = 500;

async fn child(
    state: &Arc<AppState>,
    index: &ExecutionIndex,
    action: &str,
    input: Value,
) -> RpcReply {
    state
        .hub
        .call(
            &index.node_id,
            NodeOperation::Brain {
                execution: index.execution_ref(),
                action: action.into(),
                input,
            },
        )
        .await
}

pub(in crate::api::brain_runs) async fn snapshot(
    state: &Arc<AppState>,
    id: &str,
) -> std::result::Result<LayeredSnapshot, RpcReply> {
    let reply = runs::call(state, id, "snapshot", Value::Null).await;
    if reply.status >= 300 {
        return Err(reply);
    }
    serde_json::from_value(reply.body).map_err(internal)
}

/// Read through the snapshot watermark; incomplete history is an error.
pub(super) async fn events(
    state: &Arc<AppState>,
    id: &str,
    watermark: u64,
) -> std::result::Result<Vec<LayeredEvent>, RpcReply> {
    let mut after = 0;
    let mut selected = Vec::new();
    while after < watermark {
        let reply = runs::call(
            state,
            id,
            "events",
            json!({"after":after,"limit":EVENT_PAGE}),
        )
        .await;
        if reply.status >= 300 {
            return Err(reply);
        }
        let events = page(&reply.body).ok_or_else(|| internal("invalid layered event page"))?;
        let next = events.last().map(|event| event.seq).unwrap_or(after);
        if next <= after {
            return Err(internal(
                "layered event history ended before the snapshot watermark",
            ));
        }
        selected.extend(events.into_iter().filter(|event| event.seq <= watermark));
        after = next;
    }
    Ok(selected)
}

fn page(body: &Value) -> Option<Vec<LayeredEvent>> {
    serde_json::from_value(body.get("events")?.clone()).ok()
}

pub(super) async fn summary(state: &Arc<AppState>, index: &ExecutionIndex) -> Option<String> {
    let reply = child(state, index, "layered_summary", Value::Null).await;
    if reply.status >= 300 {
        return None;
    }
    match &reply.body["summary"] {
        Value::String(summary) => Some(summary.clone()),
        _ => None,
    }
}

/// One JSON pointer read of a previous act; the child owner is the only reader.
pub(super) async fn output(
    state: &Arc<AppState>,
    index: &ExecutionIndex,
    path: &str,
) -> Result<Value> {
    let reply = child(state, index, "layered_output", json!({"path":path})).await;
    if reply.status >= 300 {
        anyhow::bail!("referenced execution output unavailable: {}", reply.body);
    }
    reply
        .body
        .get("value")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("referenced output response has no value"))
}

pub(super) fn internal(error: impl std::fmt::Display) -> RpcReply {
    RpcReply::error(500, error.to_string())
}
