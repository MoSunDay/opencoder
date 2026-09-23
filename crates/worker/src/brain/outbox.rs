use crate::{journal::Record, Worker};
use anyhow::{ensure, Result};
use opencoder_core::fleet::*;
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
        if super::v4::parent::reports(&record.assignment.request.input) {
            // Leaf children and nested runs share one terminal frame and one
            // acknowledgement fence; a nested run also publishes its own
            // projection frames for control.
            if let Some(reporting) = super::v4::parent::reporting(&record.assignment.request.input)?
            {
                if let Some(terminal) = super::v4::parent::terminal(&record, &reporting)? {
                    if record.annotations[super::v4::parent::ACK]
                        .as_u64()
                        .unwrap_or(0)
                        < terminal.source_sequence
                    {
                        frames.push(frame(&record, "layered_terminal", json!(terminal)));
                    }
                }
            }
            if super::v4::parent::root(&record.assignment.request.input) {
                frames.extend(super::v4::frames(worker, &record).await?);
            }
        } else if record.assignment.index.kind == ExecutionKind::Brain
            && record.assignment.request.input["schema_version"] == 6
        {
            frames.extend(super::v4::frames(worker, &record).await?);
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
    ensure!(
        super::v4::parent::reports(&record.assignment.request.input),
        "execution has no layered parent"
    );
    let key = super::v4::parent::ACK;
    if sequence > record.annotations[key].as_u64().unwrap_or(0) {
        if !record.annotations.is_object() {
            record.annotations = json!({});
        }
        record.annotations[key] = json!(sequence);
        journal.save(record)?;
    }
    Ok(RpcReply::ok(json!({"acknowledged":sequence})))
}
