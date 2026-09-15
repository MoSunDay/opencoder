use crate::Worker;
use anyhow::{Context, Result};
use base64::Engine;
use opencoder_core::fleet::*;
use opencoder_store::TodoWorkflowRecord;
use opencoder_todos::{review::rerun, types::*};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) async fn snapshot(
    worker: &Worker,
    id: &str,
) -> Result<Option<(TodoWorkflowRecord, i64)>> {
    let store = &worker.inner.state.store;
    for _ in 0..3 {
        let before = store.last_todo_event_seq(id).await?;
        let row = store.get_todo_workflow(id).await?;
        let after = store.last_todo_event_seq(id).await?;
        if before == after {
            return Ok(row.map(|row| (row, after)));
        }
    }
    anyhow::bail!("workflow changed during snapshot; retry the read");
}

pub(super) async fn query(worker: &Worker, id: &str, input: Value) -> Result<RpcReply> {
    let Some((record, head)) = snapshot(worker, id).await? else {
        let reply = super::super::query::inspect(
            worker,
            &ExecutionRef {
                id: id.into(),
                kind: ExecutionKind::Todos,
            },
        )
        .await?;
        if reply.status != 200 {
            return Ok(reply);
        }
        if reply.body["workflow_initializing"] == true {
            return Ok(RpcReply::ok(json!({"initializing":true})));
        }
        return Ok(RpcReply::error(
            409,
            format!(
                "workflow initialization {}: {}",
                reply.body["workflow_initialization"]
                    .as_str()
                    .unwrap_or("inconsistent"),
                reply.body["error"]
                    .as_str()
                    .unwrap_or("workflow data is unavailable")
            ),
        ));
    };
    if input["generation"]
        .as_i64()
        .is_some_and(|g| g != record.generation)
    {
        return Ok(RpcReply::error(
            409,
            "workflow changed; reload the review snapshot",
        ));
    }
    let spec: WorkflowSpec = serde_json::from_value(record.spec_json.clone())?;
    let state: WorkflowState = serde_json::from_value(record.state_json.clone())?;
    let section = input["section"].as_str().unwrap_or("overview");
    let value = match section {
        "files" => {
            json!({"files":opencoder_todos::directory::encode(&spec, spec.metadata["env"].as_str())?})
        }
        "overview" => overview(worker, &record, &spec, &state, head, &input).await?,
        "node" => {
            let Some(todo) = spec
                .todos
                .iter()
                .find(|t| Some(t.id.as_str()) == input["todo_id"].as_str())
            else {
                return Ok(RpcReply::error(404, "TODO not found"));
            };
            json!({"todo":todo,"state":state.todos.get(&todo.id),"generation":record.generation,
                "preview":rerun::preview(&spec, &state, &todo.id)?})
        }
        "history" => {
            let before = input["before_seq"]
                .as_i64()
                .unwrap_or(head.saturating_add(1));
            if before <= 0 {
                return Ok(RpcReply::error(400, "invalid event cursor"));
            }
            let page = worker
                .inner
                .state
                .store
                .todo_events_before(id, before, 200, 256 * 1024)
                .await?;
            let next = page.events.last().and_then(|e| e.seq);
            json!({"events":page.events,"more":page.more,
                "next_before_seq":if page.more {next} else {None}})
        }

        "messages" | "session_events" | "session_event_payload" => {
            let sid = input["session_id"].as_str().unwrap_or("");
            if sid != state.parent_session_id
                && !state
                    .todos
                    .values()
                    .any(|t| t.session_history.iter().any(|id| id == sid))
            {
                return Ok(RpcReply::error(
                    403,
                    "session does not belong to this workflow",
                ));
            }
            let execution = ExecutionRef {
                id: sid.into(),
                kind: ExecutionKind::Agent,
            };
            if section == "messages" {
                let cursor = MessageCursor {
                    seq: input["after_seq"].as_i64().unwrap_or(0),
                    offset: input["message_offset"].as_u64().unwrap_or(0),
                };
                if cursor.seq < 0 {
                    return Ok(RpcReply::error(400, "invalid message cursor"));
                }
                return super::super::query::messages(worker, &execution, cursor).await;
            }
            if section == "session_event_payload" {
                return super::super::query::event_payload(
                    worker,
                    EventPayloadRequest {
                        execution,
                        seq: input["after_seq"].as_i64().unwrap_or(0),
                        offset: input["offset"].as_u64().unwrap_or(0),
                    },
                )
                .await;
            }
            let after = input["after_seq"].as_i64().unwrap_or(0);
            if after < 0 {
                return Ok(RpcReply::error(400, "invalid event cursor"));
            }
            return super::super::query::events(worker, &execution, after).await;
        }
        _ => return Ok(RpcReply::error(400, "unknown review section")),
    };
    chunk(value, &input)
}

async fn overview(
    worker: &Worker,
    record: &TodoWorkflowRecord,
    spec: &WorkflowSpec,
    state: &WorkflowState,
    head: i64,
    input: &Value,
) -> Result<Value> {
    let start = input["after_ordinal"].as_u64().unwrap_or(0) as usize;
    anyhow::ensure!(start <= spec.todos.len(), "invalid TODO cursor");
    let nodes: Vec<_> = spec
        .todos
        .iter()
        .skip(start)
        .take(100)
        .map(|todo| {
            let item = state.todos.get(&todo.id)
                .with_context(|| format!("workflow state missing TODO {}", todo.id))?;
            Ok(json!({"id":todo.id,"title":todo.title.chars().take(160).collect::<String>(),
            "agent":todo.agent,"depends_on":todo.depends_on,"status":item.status,
            "attempt":item.attempt,"active_session_id":item.active_session_id,
            "milestone":state.milestones.contains(&todo.id),
            "last_error":item.last_error.as_ref().map(|e| e.chars().take(200).collect::<String>())}))
        })
        .collect::<Result<Vec<_>>>()?;
    let end = start + nodes.len();
    let (execution_status, mut controls) = {
        let journal = worker.inner.journal.lock().await;
        let execution = journal
            .records
            .get(&record.id)
            .context("execution not found")?;
        (
            execution.assignment.index.status,
            execution
                .lifecycle
                .todo_reruns
                .values()
                .map(|c| c.receipt())
                .collect::<Vec<_>>(),
        )
    };
    controls.sort_by_key(|control| control["accepted_at"].as_i64().unwrap_or(0));
    let latest = worker
        .inner
        .state
        .store
        .todo_events_page(&record.id, (head - 1).max(0), 1, 64 * 1024)
        .await?;
    Ok(
        json!({"workflow":{"id":record.id,"name":spec.name,"objective":spec.objective,
        "constraints":spec.constraints,"status":record.status,"parent_session_id":state.parent_session_id,
        "generation":record.generation,"world_epoch":state.world_epoch,"updated_at":record.updated_at,
        "terminal_reason":record.terminal_reason},"execution_status":execution_status,
        "nodes":nodes,"total":spec.todos.len(),"head_seq":head,"controls":controls,
        "next_ordinal":if end < spec.todos.len() {Some(end)} else {None},
        "latest_event":latest.events.into_iter().find(|e| e.seq == Some(head))}),
    )
}

/// Large review fields use bounded transport with a content identity on every chunk.
fn chunk(value: Value, input: &Value) -> Result<RpcReply> {
    let bytes = serde_json::to_vec(&value)?;
    if bytes.len() <= 64 * 1024 && input["offset"].as_u64().unwrap_or(0) == 0 {
        return Ok(RpcReply::ok(value));
    }
    let etag = format!("{:x}", Sha256::digest(&bytes));
    if input["etag"]
        .as_str()
        .is_some_and(|expected| expected != etag)
    {
        return Ok(RpcReply::error(
            409,
            "review content changed during chunk read",
        ));
    }
    let start = input["offset"].as_u64().unwrap_or(0) as usize;
    if start > bytes.len() {
        return Ok(RpcReply::error(400, "invalid review offset"));
    }
    let end = (start + 64 * 1024).min(bytes.len());
    Ok(RpcReply::ok(
        json!({"encoding":"json-base64","etag":etag,"offset":start,
        "next_offset":end,"eof":end==bytes.len(),"total_bytes":bytes.len(),
        "bytes_b64":base64::engine::general_purpose::STANDARD.encode(&bytes[start..end])}),
    ))
}
