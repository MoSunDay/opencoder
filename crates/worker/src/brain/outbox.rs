use super::persistence;
use crate::{journal::Record, Worker};
use anyhow::{ensure, Result};
use opencoder_core::{brain::*, fleet::*};
use serde_json::{json, Value};

pub async fn frames(worker: &Worker) -> Result<Vec<NodeFrame>> {
    let records: Vec<_> = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .values()
        .cloned()
        .collect();
    let mut frames = Vec::new();
    for record in records {
        if record.assignment.index.kind == ExecutionKind::Brain {
            let Some((_, run)) = persistence::load(worker, &record.assignment.index.id).await?
            else {
                continue;
            };
            if run.phase == RunPhase::Planning {
                if let Some(candidate) = run.candidate_plan {
                    frames.push(frame(&record, "publish", json!(candidate)));
                }
            }
            for receipt in run
                .actions
                .values()
                .filter(|a| a.state == ReceiptState::Prepared)
            {
                // Pause fences dispatch at the final source emission too.
                if (receipt.kind == ActionKind::Execute && run.phase != RunPhase::Running)
                    || (receipt.kind == ActionKind::Cancel && run.phase != RunPhase::Cancelling)
                {
                    continue;
                }
                frames.push(frame(&record, "action", json!(receipt)));
            }
        } else if let Some(notice) = notice(&record)? {
            if record.annotations["brain_notice_ack"].as_u64().unwrap_or(0) < notice.sequence {
                let mut input = json!(notice);
                input["capability_id"] =
                    record.assignment.request.input["_brain"]["capability_id"].clone();
                frames.push(frame(&record, "notice", input));
            }
        }
    }
    if !frames.is_empty() {
        let cursor = worker
            .inner
            .brain_frame_cursor
            .fetch_add(64, std::sync::atomic::Ordering::Relaxed) as usize;
        let len = frames.len();
        frames.rotate_left(cursor % len);
        frames.truncate(64);
    }
    Ok(frames)
}

fn frame(record: &Record, action: &str, input: Value) -> NodeFrame {
    NodeFrame::Brain {
        execution: record.assignment.index.execution_ref(),
        action: action.into(),
        input,
    }
}

pub fn notice(record: &Record) -> Result<Option<BrainNotice>> {
    let Some(parent) = record.assignment.request.input["_brain"].get("parent") else {
        return Ok(None);
    };
    let sequence = record.events.last().and_then(|e| e.seq).unwrap_or(0) as u64;
    let status = match record.assignment.index.status {
        ExecutionStatus::Pending => StepStatus::Queued,
        ExecutionStatus::Running => StepStatus::Running,
        ExecutionStatus::Done => StepStatus::Succeeded,
        ExecutionStatus::Cancelled => StepStatus::Cancelled,
        ExecutionStatus::Error | ExecutionStatus::Interrupted => StepStatus::Failed,
        ExecutionStatus::Idle => StepStatus::Failed,
        ExecutionStatus::Cancelling => return Ok(None),
    };
    Ok(Some(BrainNotice {
        parent: serde_json::from_value(parent.clone())?,
        execution: record.assignment.index.execution_ref(),
        node_id: record.assignment.index.node_id.clone(),
        sequence,
        status,
        output: record
            .result
            .get("brain_output")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?,
        error: record.error.clone(),
        at_ms: persistence::now(),
    }))
}

pub async fn ack(worker: &Worker, reference: &ExecutionRef, input: Value) -> Result<RpcReply> {
    let mut journal = worker.inner.journal.lock().await;
    let Some(mut record) = journal.records.get(&reference.id).cloned() else {
        return Ok(RpcReply::error(404, "source execution missing"));
    };
    ensure!(
        record.assignment.index.kind == reference.kind,
        "source execution kind mismatch"
    );
    let sequence = input["sequence"].as_u64().unwrap_or(0);
    ensure!(
        sequence <= record.events.last().and_then(|e| e.seq).unwrap_or(0) as u64,
        "invalid acknowledgement cursor"
    );
    if sequence > record.annotations["brain_notice_ack"].as_u64().unwrap_or(0) {
        if !record.annotations.is_object() {
            record.annotations = json!({});
        }
        record.annotations["brain_notice_ack"] = json!(sequence);
        journal.save(record)?;
    }
    Ok(RpcReply::ok(json!({"acknowledged":sequence})))
}
