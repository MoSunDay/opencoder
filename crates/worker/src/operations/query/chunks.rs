use super::view::bounded_reply;
use crate::Worker;
use anyhow::Result;
use base64::Engine;
use opencoder_core::fleet::*;
use serde_json::Value;
use std::io::{Read, Seek};

pub(in crate::operations) async fn event_payload(
    worker: &Worker,
    request: EventPayloadRequest,
) -> Result<RpcReply> {
    if let Some(reply) = crate::operations::validate_reference(worker, &request.execution).await? {
        return Ok(reply);
    }
    if request.seq <= 0 {
        return Ok(RpcReply::error(400, "event sequence must be positive"));
    }
    if request.execution.kind == ExecutionKind::Project && request.execution.id.starts_with("prun-")
    {
        let mut reply = super::project::file(
            worker,
            &request.execution.id,
            &format!("event-{}.json", request.seq),
            request.offset,
        )?;
        if reply.status == 200 {
            reply.body["seq"] = serde_json::json!(request.seq);
            reply.body["encoding"] = serde_json::json!("base64");
        }
        return Ok(reply);
    }
    let id = request.execution.id.as_str();
    let chunk = if request.execution.kind == ExecutionKind::Todos {
        worker
            .inner
            .state
            .store
            .todo_event_payload_chunk(id, request.seq, request.offset, EVENT_CHUNK_BYTES)
            .await?
    } else {
        let session_id = if request.execution.kind == ExecutionKind::Project {
            worker
                .inner
                .state
                .project
                .require()?
                .projects
                .get_todo_run_summary(id)
                .await?
                .and_then(|run| run.session_id)
                .unwrap_or_else(|| id.to_owned())
        } else {
            id.to_owned()
        };
        if worker
            .inner
            .state
            .store
            .get_session(&session_id)
            .await?
            .is_some()
        {
            worker
                .inner
                .state
                .store
                .event_payload_chunk(&session_id, request.seq, request.offset, EVENT_CHUNK_BYTES)
                .await?
        } else {
            let event = worker
                .inner
                .journal
                .lock()
                .await
                .records
                .get(id)
                .and_then(|record| {
                    record
                        .events
                        .iter()
                        .find(|event| event.seq == Some(request.seq))
                        .cloned()
                });
            event
                .map(|event| value_chunk(&event.data, request.offset, EVENT_CHUNK_BYTES))
                .transpose()?
        }
    };
    let Some(chunk) = chunk else {
        return Ok(RpcReply::error(404, "event payload not found"));
    };
    let next_offset = request.offset + chunk.bytes.len() as u64;
    bounded_reply(serde_json::to_value(EventPayloadChunk {
        seq: request.seq,
        offset: request.offset,
        next_offset,
        total_bytes: chunk.total_bytes,
        eof: next_offset >= chunk.total_bytes,
        encoding: "base64".into(),
        bytes_b64: base64::engine::general_purpose::STANDARD.encode(chunk.bytes),
    })?)
}

pub(in crate::operations) async fn detail_field(
    worker: &Worker,
    request: DetailFieldRequest,
) -> Result<RpcReply> {
    if let Some(reply) = crate::operations::validate_reference(worker, &request.execution).await? {
        return Ok(reply);
    }
    let id = request.execution.id.as_str();
    if let Some(name) = request.field.strip_prefix("archive.") {
        if request.execution.kind != ExecutionKind::Project || !id.starts_with("prun-") {
            return Ok(RpcReply::error(400, "archive requires a project run"));
        }
        return super::project::file(worker, id, name, request.offset);
    }
    if let Some(field) = request.field.strip_prefix("workflow.") {
        let chunk = worker
            .inner
            .state
            .store
            .todo_workflow_field_chunk(id, field, request.offset, EVENT_CHUNK_BYTES)
            .await?;
        return detail_chunk_reply(request.field, request.offset, chunk, "utf8-base64");
    }
    if let Some(rest) = request.field.strip_prefix("todo.item.") {
        let Some((todo_id, field)) = rest.rsplit_once('.') else {
            return Ok(RpcReply::error(400, "invalid TODO item field"));
        };
        let chunk = worker
            .inner
            .state
            .store
            .todo_item_field_chunk(id, todo_id, field, request.offset, EVENT_CHUNK_BYTES)
            .await?;
        return detail_chunk_reply(request.field, request.offset, chunk, "utf8-base64");
    }
    if let Some(rest) = request.field.strip_prefix("project.") {
        if request.execution.kind != ExecutionKind::Project {
            return Ok(RpcReply::error(
                400,
                "project field requires a project execution",
            ));
        }
        let parts = rest.split('.').collect::<Vec<_>>();
        if parts.len() != 3 {
            return Ok(RpcReply::error(400, "invalid project detail field"));
        }
        let owner_id = if let Some(owner) = id.strip_prefix("project-") {
            owner.to_string()
        } else if id.starts_with("prun-") && parts[0] == "run" && parts[1] == id {
            super::project::run(worker, id).await?.todo_id
        } else {
            return Ok(RpcReply::error(
                400,
                "project field does not belong to this execution",
            ));
        };
        let chunk = worker
            .inner
            .state
            .project
            .require()?
            .projects
            .project_text_chunk(
                parts[0],
                &owner_id,
                parts[1],
                parts[2],
                request.offset,
                EVENT_CHUNK_BYTES,
            )
            .await?;
        return detail_chunk_reply(request.field, request.offset, chunk, "utf8-base64");
    }
    if request.field.starts_with("team.turn.") {
        let chunk = team_artifact_chunk(worker, &request, EVENT_CHUNK_BYTES).await?;
        return detail_chunk_reply(request.field, request.offset, chunk, "json-base64");
    }
    let record = worker.inner.journal.lock().await.records.get(id).cloned();
    let value = match request.field.as_str() {
        "request.input" => record
            .as_ref()
            .map(|row| super::view::public_input(&row.assignment.request.input)),
        "definition" => record
            .as_ref()
            .and_then(|row| row.assignment.definition.clone()),
        "result" => record.as_ref().map(|row| row.result.clone()),
        "team.topic" => {
            let legacy = worker.inner.journal.lock().await.uses_legacy(id);
            let Some(path) = team_topic_path(worker, &request.execution, record.as_ref(), legacy)?
            else {
                return Ok(RpcReply::error(404, "detail field not found"));
            };
            let chunk = read_file_chunk(&path, request.offset, EVENT_CHUNK_BYTES)?;
            return detail_chunk_reply(request.field, request.offset, chunk, "json-base64");
        }
        _ => return Ok(RpcReply::error(400, "unsupported detail field")),
    };
    let Some(value) = value else {
        return Ok(RpcReply::error(404, "detail field not found"));
    };
    let chunk = value_chunk(&value, request.offset, EVENT_CHUNK_BYTES)?;
    let next_offset = request.offset + chunk.bytes.len() as u64;
    bounded_reply(serde_json::to_value(DetailFieldChunk {
        field: request.field,
        offset: request.offset,
        next_offset,
        total_bytes: chunk.total_bytes,
        eof: next_offset >= chunk.total_bytes,
        encoding: "json-base64".into(),
        bytes_b64: base64::engine::general_purpose::STANDARD.encode(chunk.bytes),
    })?)
}

async fn team_artifact_chunk(
    worker: &Worker,
    request: &DetailFieldRequest,
    max_bytes: usize,
) -> Result<Option<opencoder_store::PayloadChunkRecord>> {
    if !matches!(
        request.execution.kind,
        ExecutionKind::Team | ExecutionKind::System
    ) {
        anyhow::bail!("team detail field requires a Team execution");
    }
    let record = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(&request.execution.id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("team execution not found"))?;
    let name = if request.execution.kind == ExecutionKind::System {
        "system"
    } else {
        record
            .assignment
            .definition
            .as_ref()
            .and_then(|value| value["name"].as_str())
            .ok_or_else(|| anyhow::anyhow!("team execution has no team name"))?
    };
    let legacy = worker
        .inner
        .journal
        .lock()
        .await
        .uses_legacy(&request.execution.id);
    let root = if legacy {
        worker.inner.layout.legacy_team_dir(&request.execution.id)?
    } else {
        worker
            .inner
            .layout
            .team_state_dir(request.execution.kind, &request.execution.id)?
    };
    let parts = request.field.split('.').collect::<Vec<_>>();
    let path = match parts.as_slice() {
        ["team", "turn", turn, "plan"] => {
            opencoder_team::layout::plan_file(&root, name, &request.execution.id, turn.parse()?)?
        }
        ["team", "turn", turn, "sub", sub, "summary"] => opencoder_team::layout::summary_file(
            &root,
            name,
            &request.execution.id,
            turn.parse()?,
            sub.parse()?,
        )?,
        ["team", "turn", turn, "sub", sub, "result", member] => {
            opencoder_team::layout::result_file(
                &root,
                name,
                &request.execution.id,
                turn.parse()?,
                sub.parse()?,
                member,
            )?
        }
        _ => anyhow::bail!("invalid team detail field"),
    };
    read_file_chunk(&path, request.offset, max_bytes)
}

fn read_file_chunk(
    path: &std::path::Path,
    offset: u64,
    max_bytes: usize,
) -> Result<Option<opencoder_store::PayloadChunkRecord>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let total_bytes = file.metadata()?.len();
    if total_bytes > opencoder_team::MAX_FILE_BYTES as u64 {
        anyhow::bail!("team detail file exceeds its size limit");
    }
    if offset > total_bytes {
        anyhow::bail!("team detail offset exceeds total bytes");
    }
    file.seek(std::io::SeekFrom::Start(offset))?;
    let mut bytes = vec![0; max_bytes.clamp(1, EVENT_CHUNK_BYTES)];
    let read = file.read(&mut bytes)?;
    bytes.truncate(read);
    Ok(Some(opencoder_store::PayloadChunkRecord {
        total_bytes,
        bytes,
    }))
}

fn detail_chunk_reply(
    field: String,
    offset: u64,
    chunk: Option<opencoder_store::PayloadChunkRecord>,
    encoding: &str,
) -> Result<RpcReply> {
    let Some(chunk) = chunk else {
        return Ok(RpcReply::error(404, "detail field not found"));
    };
    let next_offset = offset + chunk.bytes.len() as u64;
    bounded_reply(serde_json::to_value(DetailFieldChunk {
        field,
        offset,
        next_offset,
        total_bytes: chunk.total_bytes,
        eof: next_offset >= chunk.total_bytes,
        encoding: encoding.into(),
        bytes_b64: base64::engine::general_purpose::STANDARD.encode(chunk.bytes),
    })?)
}

fn team_topic_path(
    worker: &Worker,
    execution: &ExecutionRef,
    record: Option<&crate::journal::Record>,
    legacy: bool,
) -> Result<Option<std::path::PathBuf>> {
    let Some(record) = record else {
        return Ok(None);
    };
    if !matches!(execution.kind, ExecutionKind::Team | ExecutionKind::System) {
        return Ok(None);
    }
    let name = if execution.kind == ExecutionKind::System {
        "system"
    } else {
        record
            .assignment
            .definition
            .as_ref()
            .and_then(|value| value["name"].as_str())
            .ok_or_else(|| anyhow::anyhow!("team execution has no team name"))?
    };
    let root = if legacy {
        worker.inner.layout.legacy_team_dir(&execution.id)?
    } else {
        worker
            .inner
            .layout
            .team_state_dir(execution.kind, &execution.id)?
    };
    Ok(Some(opencoder_team::layout::topic_file(
        &root,
        name,
        &execution.id,
    )?))
}

fn value_chunk(
    value: &Value,
    offset: u64,
    max_bytes: usize,
) -> Result<opencoder_store::PayloadChunkRecord> {
    struct SliceWriter {
        offset: usize,
        position: usize,
        bytes: Vec<u8>,
        limit: usize,
    }
    impl std::io::Write for SliceWriter {
        fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
            let start = self.position;
            self.position = self.position.saturating_add(input.len());
            if self.bytes.len() < self.limit && self.position > self.offset {
                let from = self.offset.saturating_sub(start).min(input.len());
                let take = (self.limit - self.bytes.len()).min(input.len() - from);
                self.bytes.extend_from_slice(&input[from..from + take]);
            }
            Ok(input.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let offset = usize::try_from(offset)?;
    let mut writer = SliceWriter {
        offset,
        position: 0,
        bytes: Vec::with_capacity(max_bytes),
        limit: max_bytes,
    };
    serde_json::to_writer(&mut writer, value)?;
    if offset > writer.position {
        anyhow::bail!("event payload offset exceeds total bytes");
    }
    Ok(opencoder_store::PayloadChunkRecord {
        total_bytes: writer.position as u64,
        bytes: writer.bytes,
    })
}
