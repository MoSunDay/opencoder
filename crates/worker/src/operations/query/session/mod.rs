//! Reads during the gap between durable acceptance and session initialization.
use crate::{journal::Record, Worker};
use anyhow::Result;
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus, RpcReply};
use serde_json::{json, Value};

/// A session row can exist before its first drain, or while a continuation
/// waits for a slot. Keep clients subscribed until the owning execution settles.
pub(in crate::operations) fn with_pending_work(
    mut reply: RpcReply,
    status: ExecutionStatus,
) -> RpcReply {
    if (200..300).contains(&reply.status)
        && matches!(
            status,
            ExecutionStatus::Pending | ExecutionStatus::Running | ExecutionStatus::Cancelling
        )
    {
        reply.body["draining"] = json!(true);
    }
    reply
}

pub(in crate::operations) fn summary(record: &Record) -> Value {
    let assignment = &record.assignment;
    let input = &assignment.request.input;
    json!({
        "id": assignment.index.id,
        "kind": assignment.index.kind,
        "agent": assignment.request.target.as_deref().unwrap_or("act"),
        "title": input["title"].as_str().or(input["prompt"].as_str()).map(|text| text.chars().take(120).collect::<String>()),
        "created_at": assignment.index.created_at,
        "updated_at": assignment.index.created_at,
        "status": assignment.index.status,
    })
}

/// Only the node's accepted journal can establish an initializing session.
/// Unknown IDs still fail through the ordinary reference validation.
pub(in crate::operations) async fn before_ready(
    worker: &Worker,
    id: &str,
    tail: &str,
) -> Result<Option<RpcReply>> {
    if !matches!(tail, "" | "seq" | "messages" | "transcript")
        || worker.inner.state.store.get_session(id).await?.is_some()
    {
        return Ok(None);
    }
    let record = worker.inner.journal.lock().await.records.get(id).cloned();
    let Some(record) = record.filter(|record| {
        matches!(
            record.assignment.index.kind,
            ExecutionKind::Agent | ExecutionKind::Operator | ExecutionKind::Maintenance
        )
    }) else {
        return Ok(None);
    };
    let status = record.assignment.index.status;
    if !matches!(
        status,
        ExecutionStatus::Pending | ExecutionStatus::Running | ExecutionStatus::Cancelling
    ) {
        if worker.inner.state.store.get_session(id).await?.is_some() {
            return Ok(None);
        }
        return Ok(Some(RpcReply::error(
            409,
            record.error.unwrap_or_else(|| {
                format!(
                    "execution {} before session initialization",
                    status.as_str()
                )
            }),
        )));
    }
    let body = match tail {
        "seq" => json!({"id": id, "seq": 0}),
        "transcript" => json!({"chunks": [], "more": false, "initializing": true}),
        _ => json!({
            "id": id, "meta": summary(&record), "messages": [],
            "draining": true, "initializing": true, "status": status,
        }),
    };
    Ok(Some(RpcReply::ok(body)))
}
