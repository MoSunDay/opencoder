//! Run-scoped readers never infer a run boundary from a whole resumed session.
use crate::Worker;
use anyhow::{Context, Result};
use base64::Engine;
use opencoder_core::fleet::*;
use opencoder_store::{ProjectRunText, ProjectTodoRunSummary};
use serde_json::{json, Value};

pub async fn run(worker: &Worker, id: &str) -> Result<ProjectTodoRunSummary> {
    worker
        .inner
        .state
        .project
        .require()?
        .projects
        .get_todo_run_summary(id)
        .await?
        .context("project run not found")
}
pub async fn manifest(worker: &Worker, run: &ProjectTodoRunSummary) -> Result<Value> {
    let Some(value) = &run.trace_manifest else {
        return Ok(Value::Null);
    };
    match value {
        ProjectRunText::Text(text) => Ok(serde_json::from_str(text)?),
        ProjectRunText::Omitted { .. } => {
            let projects = worker.inner.state.project.require()?.projects.clone();
            let mut bytes = Vec::new();
            let mut offset = 0;
            loop {
                let chunk = projects
                    .project_text_chunk(
                        "run",
                        &run.todo_id,
                        &run.id,
                        "trace_manifest",
                        offset,
                        65536,
                    )
                    .await?
                    .context("run manifest missing")?;
                offset += chunk.bytes.len() as u64;
                bytes.extend(chunk.bytes);
                if offset >= chunk.total_bytes {
                    break;
                }
            }
            Ok(serde_json::from_slice(&bytes)?)
        }
    }
}
pub fn root(worker: &Worker, id: &str) -> Result<std::path::PathBuf> {
    opencoder_project::trace::archive::run_root(
        &opencoder_project::trace::root(worker.inner.state.project.require()?.as_ref()),
        id,
    )
}
pub async fn inspect(worker: &Worker, run: ProjectTodoRunSummary) -> Result<RpcReply> {
    let trace = manifest(worker, &run).await?;
    let status = match run.status {
        opencoder_store::ProjectTodoRunStatus::Running => {
            if worker
                .inner
                .active
                .lock()
                .await
                .contains_key(&format!("project-{}", run.todo_id))
            {
                "running"
            } else {
                "interrupted"
            }
        }
        opencoder_store::ProjectTodoRunStatus::Done => "done",
        opencoder_store::ProjectTodoRunStatus::Cancelled => "cancelled",
        _ => "error",
    };
    let index = json!({"id":run.id,"kind":"project","node_id":worker.inner.registration.id,"created_at":run.created_at,"status":status});
    super::view::bounded_reply(
        json!({"execution":index,"run":run,"replay":trace,"retention":if run.input_snapshot.is_none(){"incomplete_history"}else if trace["complete"]==true{"complete"}else{"partial"}}),
    )
}
pub async fn messages(worker: &Worker, id: &str, mut cursor: MessageCursor) -> Result<RpcReply> {
    let run = run(worker, id).await?;
    let trace = manifest(worker, &run).await?;
    let Some(start) = trace["messages_after"].as_i64() else {
        return Ok(RpcReply::error(
            409,
            "historical run has no message boundaries",
        ));
    };
    let sid = run.session_id.context("run session unavailable")?;
    let mut end = match trace["messages_through"].as_i64() {
        Some(end) => end,
        None if run.status == opencoder_store::ProjectTodoRunStatus::Running => {
            worker.inner.state.store.last_message_seq(&sid).await?
        }
        _ => {
            return Ok(RpcReply::error(
                409,
                "interrupted run message boundary unavailable",
            ))
        }
    };
    // If finalization raced this read, use the newly sealed boundary. The
    // provisional end was read first, so it also remains safe if sealing
    // happens after this second lookup.
    if trace["messages_through"].is_null() {
        let latest = self::run(worker, id).await?;
        if let Some(sealed) = manifest(worker, &latest).await?["messages_through"].as_i64() {
            end = end.min(sealed);
        }
    }
    cursor.seq = cursor.seq.max(start + 1);
    if cursor.seq > end {
        return Ok(RpcReply::ok(json!({"chunks":[],"more":false})));
    }
    let mut page =
        super::pages::message_page_with_budget(worker, &sid, cursor, MESSAGE_PAGE_RAW_BYTES)
            .await?;
    page.chunks.retain(|c| c.seq <= end);
    page.next_cursor = page.next_cursor.filter(|c| c.seq <= end);
    page.more = page.next_cursor.is_some();
    super::view::bounded_reply(json!(page))
}
pub async fn events(worker: &Worker, id: &str, after: i64) -> Result<RpcReply> {
    let run = run(worker, id).await?;
    let root = root(worker, id)?;
    if run.input_snapshot.is_none() {
        return Ok(RpcReply::error(
            409,
            "historical run has no event boundaries",
        ));
    }
    let mut events = Vec::new();
    let mut bytes = 0;
    for seq in after.max(0).saturating_add(1)..=after.max(0).saturating_add(200) {
        let path = root.join(format!("event-{seq}.meta.json"));
        if !path.exists() {
            break;
        }
        let mut event: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let payload = root.join(format!("event-{seq}.json"));
        let size = std::fs::metadata(&payload)?.len();
        event["data"] = if size > EVENT_CHUNK_BYTES as u64 {
            json!({"omitted":true,"read_via":"event_payload","seq":seq,"total_bytes":size})
        } else {
            serde_json::from_slice(&std::fs::read(payload)?)?
        };
        let size = serde_json::to_vec(&event)?.len();
        if bytes + size > QUERY_RESPONSE_BYTES - 65536 {
            break;
        }
        bytes += size;
        events.push(event);
    }
    let last = events
        .last()
        .and_then(|e| e["seq"].as_i64())
        .unwrap_or(after.max(0));
    let more = root
        .join(format!("event-{}.meta.json", last.saturating_add(1)))
        .exists();
    super::view::bounded_reply(
        json!({"events":events,"more":more,"finished":run.status.is_terminal()}),
    )
}
pub fn chunk_reply(
    field: &str,
    offset: u64,
    chunk: Option<opencoder_store::PayloadChunkRecord>,
) -> Result<RpcReply> {
    let Some(chunk) = chunk else {
        return Ok(RpcReply::error(404, "run file unavailable"));
    };
    let next = offset + chunk.bytes.len() as u64;
    super::view::bounded_reply(
        json!({"field":field,"offset":offset,"next_offset":next,"total_bytes":chunk.total_bytes,"eof":next>=chunk.total_bytes,"encoding":"utf8-base64","bytes_b64":base64::engine::general_purpose::STANDARD.encode(chunk.bytes)}),
    )
}
pub fn file(worker: &Worker, id: &str, name: &str, offset: u64) -> Result<RpcReply> {
    let chunk = opencoder_project::trace::archive::file_chunk(
        &root(worker, id)?,
        name,
        offset,
        EVENT_CHUNK_BYTES,
    )?;
    chunk_reply(name, offset, chunk)
}
pub async fn artifact(worker: &Worker, request: ArtifactRequest) -> Result<RpcReply> {
    let run = run(worker, &request.execution.id).await?;
    let trace = manifest(worker, &run).await?;
    let Some(entry) = trace["artifacts"].as_array().and_then(|a| {
        a.iter()
            .find(|a| a["id"] == request.step && a["name"] == request.file)
    }) else {
        return Ok(RpcReply::error(404, "registered artifact unavailable"));
    };
    let version = entry["sha256"]
        .as_str()
        .context("artifact digest missing")?;
    if request.version.as_deref().is_some_and(|v| v != version) {
        return Ok(RpcReply::error(409, "artifact version mismatch"));
    }
    let name = entry["file"].as_str().context("artifact file missing")?;
    let chunk = opencoder_project::trace::archive::file_chunk(
        &root(worker, &run.id)?,
        name,
        request.offset,
        ARTIFACT_CHUNK_BYTES,
    )?
    .context("artifact file missing")?;
    let next = request.offset + chunk.bytes.len() as u64;
    super::view::bounded_reply(
        json!({"step":request.step,"file":request.file,"offset":request.offset,"next_offset":next,"total_bytes":chunk.total_bytes,"version":version,"eof":next>=chunk.total_bytes,"encoding":"base64","bytes_b64":base64::engine::general_purpose::STANDARD.encode(chunk.bytes)}),
    )
}
