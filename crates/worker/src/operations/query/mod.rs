mod chunks;
mod inspect;
mod pages;
pub(in crate::operations) mod project;
mod runner;
#[cfg(test)]
mod tests;
mod view;

use crate::Worker;
use anyhow::Result;
use axum::{body::Body, http::Request};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use tower::ServiceExt;

pub(super) use chunks::{detail_field, event_payload};
pub(super) use inspect::inspect;
pub(super) use pages::{messages, project_runs, team_turns, todo_items};

pub(crate) async fn read_reply(response: axum::response::Response) -> Result<RpcReply> {
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), MAX_FRAME_BYTES).await?;
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"error":String::from_utf8_lossy(&bytes)}))
    };
    Ok(RpcReply { status, body })
}
pub(crate) async fn native(
    worker: &Worker,
    method: &str,
    path: &str,
    body: Value,
) -> Result<RpcReply> {
    let text = if body.is_null() {
        String::new()
    } else {
        serde_json::to_string(&body)?
    };
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(text))?;
    read_reply(
        opencoder_web::build_app(worker.inner.state.clone(), None, false)
            .oneshot(request)
            .await?,
    )
    .await
}
pub(super) async fn accepted_request(
    worker: &Worker,
    execution: &ExecutionRef,
) -> Result<RpcReply> {
    if !valid_id(&execution.id) {
        return Ok(RpcReply::error(400, "invalid execution id"));
    }
    let record = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(&execution.id)
        .cloned();
    let Some(record) = record else {
        return Ok(RpcReply::error(404, "accepted request not found"));
    };
    if record.assignment.request.kind != execution.kind
        || record.assignment.index.kind != execution.kind
    {
        return Ok(RpcReply::error(409, "execution kind mismatch"));
    }
    let body = json!({
        "id": record.assignment.request.id,
        "kind": record.assignment.request.kind,
        "receipt": record.assignment.request.input.get("brain_receipt"),
    });
    if serde_json::to_vec(&body)?.len() > MAX_FRAME_BYTES - 2048 {
        return Ok(RpcReply::error(
            413,
            "accepted request receipt is too large",
        ));
    }
    Ok(RpcReply::ok(body))
}

pub(super) async fn events(
    worker: &Worker,
    execution: &ExecutionRef,
    after: i64,
) -> Result<RpcReply> {
    if let Some(reply) = super::validate_reference(worker, execution).await? {
        return Ok(reply);
    }
    if execution.kind == ExecutionKind::Project && execution.id.starts_with("prun-") {
        return project::events(worker, &execution.id, after).await;
    }
    let id = execution.id.as_str();
    let run = worker
        .inner
        .state
        .project
        .require()?
        .projects
        .get_todo_run_summary(id)
        .await?;
    let id = run
        .as_ref()
        .and_then(|r| r.session_id.as_deref())
        .unwrap_or(id);
    let record = worker.inner.journal.lock().await.records.get(id).cloned();
    let is_session = record.as_ref().is_none_or(|r| {
        matches!(
            r.assignment.request.kind,
            ExecutionKind::Agent
                | ExecutionKind::Maintenance
                | ExecutionKind::Operator
                | ExecutionKind::Dag
        )
    });
    let mut source_more = false;
    let mut head_seq = None;
    let mut frames: Vec<Value> = if is_session {
        if worker.inner.state.store.get_session(id).await?.is_none() {
            return Ok(match record {
                Some(_) => RpcReply::ok(json!({"events":[],"more":false,"head_seq":0,
                    "finished":!worker.inner.active.lock().await.contains_key(id)})),
                None => RpcReply::error(404, "session not found"),
            });
        }
        // Capture the replay watermark before reading a bounded page. A DAG can
        // have an execution ID without the session ID prefix used by legacy APIs.
        head_seq = Some(worker.inner.state.store.last_event_seq(id).await?);
        let page = match worker
            .inner
            .state
            .store
            .events_page(id, after, EVENT_PAGE_MAX, QUERY_RESPONSE_BYTES - 64 * 1024)
            .await
        {
            Ok(page) => page,
            Err(error)
                if error
                    .to_string()
                    .contains("exceeds the event page byte limit") =>
            {
                return Ok(RpcReply::error(413, error.to_string()));
            }
            Err(error) => return Err(error),
        };
        source_more = page.more;
        page.events.into_iter().map(|e| json!({"seq":e.seq,"kind":e.sse_kind.unwrap_or_else(|| "status".into()),"data":e.payload,"ts":e.ts})).collect()
    } else if record
        .as_ref()
        .is_some_and(|r| r.assignment.request.kind == ExecutionKind::Todos)
    {
        let page = match worker
            .inner
            .state
            .store
            .todo_events_page(id, after, EVENT_PAGE_MAX, QUERY_RESPONSE_BYTES - 64 * 1024)
            .await
        {
            Ok(page) => page,
            Err(error)
                if error
                    .to_string()
                    .contains("exceeds the event page byte limit") =>
            {
                return Ok(RpcReply::error(413, error.to_string()));
            }
            Err(error) => return Err(error),
        };
        source_more = page.more;
        page.events
            .into_iter()
            .map(|e| json!({"seq":e.seq,"kind":e.kind,"data":e.payload,"ts":e.ts}))
            .collect()
    } else {
        record
            .as_ref()
            .unwrap()
            .events
            .iter()
            .filter(|e| e.seq.is_some_and(|seq| seq > after))
            .map(|e| json!(e))
            .collect()
    };
    let mut bytes = 0usize;
    let mut count = 0;
    for frame in &frames {
        let size = serde_json::to_vec(frame)?.len();
        if count == 200 || bytes + size > MAX_FRAME_BYTES - 2048 {
            break;
        }
        bytes += size;
        count += 1;
    }
    if count == 0 && !frames.is_empty() {
        return Ok(RpcReply::error(
            413,
            "event exceeds the control frame limit",
        ));
    }
    let more = source_more || frames.len() > count;
    frames.truncate(count);
    let draining = worker
        .inner
        .state
        .handles
        .lock()
        .await
        .get(id)
        .is_some_and(|h| h.draining.load(std::sync::atomic::Ordering::SeqCst));
    let finished = !draining && !worker.inner.active.lock().await.contains_key(id);
    let mut body = json!({"events":frames,"more":more,"finished":finished});
    if let Some(seq) = head_seq {
        body["head_seq"] = json!(seq);
    }
    Ok(RpcReply::ok(body))
}
