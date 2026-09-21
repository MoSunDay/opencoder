//! Bounded reads of the node-owned v4 projection.
//!
//! The run lives on the node that accepted it, so control reads it over the
//! existing brain RPC. `snapshot`/`events` mirror the v3 action names (the
//! run's own schema_version selects the payload) and `scheduler_output` /
//! `scheduler_summary` keep the bounded child reads usable; the `layered_*`
//! aliases are accepted for a node that names them explicitly. A run is only
//! admitted on a node that advertised the v4 capability, so a legacy answer
//! can never be mistaken for a v4 one.
use crate::{api::brain_runs::runs, AppState};
use anyhow::Result;
use opencoder_core::{brain::layered::*, fleet::*};
use serde_json::{json, Value};
use std::sync::Arc;

/// The node that owns a layered run answers the same projection read names as
/// its v3 counterpart: the run's own `schema_version` selects the payload, so
/// no alias is needed and a read costs exactly one round trip.
pub(super) const SNAPSHOT: &[&str] = &["snapshot"];
pub(super) const EVENTS: &[&str] = &["events"];
pub(super) const SUMMARY: &[&str] = &["layered_summary", "scheduler_summary"];
pub(super) const OUTPUT: &[&str] = &["layered_output", "scheduler_output"];
const EVENT_PAGE: u32 = 500;
const EVENT_PAGES: usize = 8;

/// First action that answers; a protocol answer (not "unsupported") wins.
fn first(replies: Vec<RpcReply>) -> RpcReply {
    let mut last = RpcReply::error(501, "layered run read was not answered");
    for reply in replies {
        if !matches!(reply.status, 400 | 404 | 501) {
            return reply;
        }
        last = reply;
    }
    last
}

async fn call(state: &Arc<AppState>, id: &str, actions: &[&str], input: Value) -> RpcReply {
    let mut replies = vec![];
    for action in actions {
        replies.push(runs::call(state, id, action, input.clone()).await);
    }
    first(replies)
}

async fn child(
    state: &Arc<AppState>,
    index: &ExecutionIndex,
    actions: &[&str],
    input: Value,
) -> RpcReply {
    let mut replies = vec![];
    for action in actions {
        replies.push(
            state
                .hub
                .call(
                    &index.node_id,
                    NodeOperation::Brain {
                        execution: index.execution_ref(),
                        action: (*action).into(),
                        input: input.clone(),
                    },
                )
                .await,
        );
    }
    first(replies)
}

pub(super) async fn snapshot(
    state: &Arc<AppState>,
    id: &str,
) -> std::result::Result<LayeredSnapshot, RpcReply> {
    let reply = call(state, id, SNAPSHOT, Value::Null).await;
    if reply.status >= 300 {
        return Err(reply);
    }
    serde_json::from_value(reply.body).map_err(internal)
}

/// Every event up to the run's watermark; a node that cannot page history
/// yields the events it did answer with.
pub(super) async fn events(state: &Arc<AppState>, id: &str, watermark: u64) -> Vec<LayeredEvent> {
    let mut after = 0;
    let mut selected = Vec::new();
    for _ in 0..EVENT_PAGES {
        if after >= watermark {
            break;
        }
        let reply = call(state, id, EVENTS, json!({"after":after,"limit":EVENT_PAGE})).await;
        if reply.status >= 300 {
            break;
        }
        let Some(events) = page(&reply.body) else {
            break;
        };
        let next = events.last().map(|event| event.seq).unwrap_or(after);
        selected.extend(events);
        if next <= after {
            break;
        }
        after = next;
    }
    selected.retain(|event| event.seq <= watermark);
    selected
}

fn page(body: &Value) -> Option<Vec<LayeredEvent>> {
    let rows = match body.get("events") {
        Some(rows) => rows.clone(),
        None if body.is_array() => body.clone(),
        None => return None,
    };
    serde_json::from_value(rows).ok()
}

/// Bounded child summary; `None` when the child cannot answer.
pub(super) async fn summary(state: &Arc<AppState>, index: &ExecutionIndex) -> Option<String> {
    let reply = child(state, index, SUMMARY, Value::Null).await;
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
    let reply = child(state, index, OUTPUT, json!({"path":path})).await;
    if reply.status >= 300 {
        anyhow::bail!("referenced execution output unavailable: {}", reply.body);
    }
    Ok(reply.body.get("value").cloned().unwrap_or(Value::Null))
}

pub(super) fn internal(error: impl std::fmt::Display) -> RpcReply {
    RpcReply::error(500, error.to_string())
}
