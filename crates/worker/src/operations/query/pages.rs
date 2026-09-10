use super::view::*;
use crate::Worker;
use anyhow::Result;
use base64::Engine;
use opencoder_core::fleet::*;
use serde_json::json;

pub(in crate::operations) async fn messages(
    worker: &Worker,
    execution: &ExecutionRef,
    cursor: MessageCursor,
) -> Result<RpcReply> {
    if let Some(reply) = crate::operations::validate_reference(worker, execution).await? {
        return Ok(reply);
    }
    if execution.kind == ExecutionKind::Project && execution.id.starts_with("prun-") {
        return super::project::messages(worker, &execution.id, cursor).await;
    }
    if !matches!(
        execution.kind,
        ExecutionKind::Agent
            | ExecutionKind::Maintenance
            | ExecutionKind::Operator
            | ExecutionKind::Dag
    ) {
        return Ok(RpcReply::error(400, "messages require a session execution"));
    }
    if worker
        .inner
        .state
        .store
        .get_session(&execution.id)
        .await?
        .is_none()
    {
        return Ok(RpcReply::error(404, "session not found"));
    }
    bounded_reply(serde_json::to_value(
        message_page(worker, &execution.id, cursor).await?,
    )?)
}

pub(in crate::operations) async fn todo_items(
    worker: &Worker,
    execution: &ExecutionRef,
    after_ordinal: Option<i64>,
) -> Result<RpcReply> {
    if let Some(reply) = crate::operations::validate_reference(worker, execution).await? {
        return Ok(reply);
    }
    if execution.kind != ExecutionKind::Todos || after_ordinal.is_some_and(|value| value < 0) {
        return Ok(RpcReply::error(
            400,
            "todo items require a valid TODO execution cursor",
        ));
    }
    let page = worker
        .inner
        .state
        .store
        .list_todo_items_page(&execution.id, after_ordinal, 100)
        .await?;
    bounded_reply(json!({
        "items": page.items,
        "next_ordinal": page.next_ordinal,
        "more": page.next_ordinal.is_some(),
    }))
}

pub(in crate::operations) async fn project_runs(
    worker: &Worker,
    execution: &ExecutionRef,
    before_version: Option<i64>,
) -> Result<RpcReply> {
    if let Some(reply) = crate::operations::validate_reference(worker, execution).await? {
        return Ok(reply);
    }
    if execution.kind != ExecutionKind::Project || before_version.is_some_and(|value| value <= 0) {
        return Ok(RpcReply::error(
            400,
            "project runs require a valid Project cursor",
        ));
    }
    let todo_id = execution
        .id
        .strip_prefix("project-")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("project execution id has no todo id"))?;
    let page = worker
        .inner
        .state
        .project
        .require()?
        .projects
        .list_todo_runs_page(todo_id, before_version, 20)
        .await?;
    bounded_reply(json!({
        "runs": page.runs,
        "next_version": page.next_version,
        "more": page.next_version.is_some(),
    }))
}

pub(in crate::operations) async fn team_turns(
    worker: &Worker,
    execution: &ExecutionRef,
    after_turn: u32,
) -> Result<RpcReply> {
    if let Some(reply) = crate::operations::validate_reference(worker, execution).await? {
        return Ok(reply);
    }
    if !matches!(execution.kind, ExecutionKind::Team | ExecutionKind::System) {
        return Ok(RpcReply::error(400, "team turns require a Team execution"));
    }
    let record = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(&execution.id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("team execution not found"))?;
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
    let legacy = worker.inner.journal.lock().await.uses_legacy(&execution.id);
    let root = if legacy {
        worker.inner.layout.legacy_team_dir(&execution.id)?
    } else {
        worker
            .inner
            .layout
            .team_state_dir(execution.kind, &execution.id)?
    };
    let page = opencoder_team::fs_store::load_topic_turns_page(
        &root,
        name,
        &execution.id,
        after_turn as usize,
        50,
    )?;
    bounded_reply(json!({
        "turns":page.turns,
        "next_turn":page.next_turn,
        "more":page.next_turn.is_some(),
    }))
}

async fn message_page(worker: &Worker, id: &str, cursor: MessageCursor) -> Result<MessagePage> {
    message_page_with_budget(worker, id, cursor, MESSAGE_PAGE_RAW_BYTES).await
}

pub(super) async fn message_page_with_budget(
    worker: &Worker,
    id: &str,
    cursor: MessageCursor,
    raw_budget: usize,
) -> Result<MessagePage> {
    let page = worker
        .inner
        .state
        .store
        .load_message_page(id, cursor, MESSAGE_CHUNK_BYTES, raw_budget)
        .await?;
    let chunks = page
        .chunks
        .into_iter()
        .map(|chunk| {
            let next_offset = chunk.offset + chunk.bytes.len() as u64;
            MessageChunk {
                seq: chunk.seq,
                role: chunk.role,
                created_at: chunk.created_at,
                offset: chunk.offset,
                next_offset,
                total_bytes: chunk.total_bytes,
                eof: next_offset >= chunk.total_bytes,
                encoding: "base64".into(),
                bytes_b64: base64::engine::general_purpose::STANDARD.encode(chunk.bytes),
            }
        })
        .collect();
    Ok(MessagePage {
        chunks,
        more: page.next_cursor.is_some(),
        next_cursor: page.next_cursor,
    })
}
