//! Bounded report selection and durable acknowledgement recovery.
mod acknowledgements;
use crate::{journal::Journal, Worker};
pub(crate) use acknowledgements::root_ack;
use anyhow::{ensure, Result};
use opencoder_core::fleet::*;
use opencoder_store::{reporting::ReportAck, Store};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Default)]
pub(crate) struct ReportCursor {
    terminal: Option<String>,
    root: Option<String>,
    dispatch: Option<crate::journal::DispatchCache>,
}

pub(crate) async fn initialize(store: &Arc<dyn Store>, journal: &mut Journal) -> Result<()> {
    let format = release::ReportStateCompatibility::current();
    store.reporting_format(format.reader, 0).await?;
    let saved = store.report_acks().await?;
    let existing: BTreeMap<_, _> = saved
        .iter()
        .map(|a| {
            (
                (a.execution_id.as_str(), a.action.as_str(), a.item.as_str()),
                a.cursor,
            )
        })
        .collect();
    for ack in journal.legacy_report_acks() {
        if !existing
            .get(&(
                ack.execution_id.as_str(),
                ack.action.as_str(),
                ack.item.as_str(),
            ))
            .is_some_and(|cursor| *cursor >= ack.cursor)
        {
            store.report_ack(&ack).await?;
        }
    }
    for ack in &saved {
        journal.apply_report_ack(ack, format.writer == 0)?;
    }
    store.reporting_recover().await?;
    store.reporting_format(format.reader, format.writer).await?;
    Ok(())
}

fn last_terminal(frames: &[NodeFrame]) -> Option<String> {
    frames.last().and_then(|frame| match frame {
        NodeFrame::Brain { execution, .. } => Some(execution.id.clone()),
        _ => None,
    })
}

pub async fn frames(worker: &Worker) -> Result<Vec<NodeFrame>> {
    let mut cursor = worker.inner.report_cursor.lock().await;
    let mut frames = worker
        .inner
        .journal
        .lock()
        .await
        .terminal_frames(cursor.terminal.as_deref(), 32)?;
    if let Some(last) = last_terminal(&frames) {
        cursor.terminal = Some(last);
    }
    let reports = worker
        .inner
        .state
        .store
        .pending_reports(cursor.root.as_deref(), (64 - frames.len()) as u32)
        .await?;
    if reports.is_empty() {
        let more = worker
            .inner
            .journal
            .lock()
            .await
            .terminal_frames(cursor.terminal.as_deref(), 64 - frames.len())?;
        for frame in more {
            let id = match &frame {
                NodeFrame::Brain { execution, .. } => &execution.id,
                _ => unreachable!(),
            };
            if !frames
                .iter()
                .any(|f| matches!(f,NodeFrame::Brain{execution,..} if &execution.id==id))
            {
                cursor.terminal = Some(id.clone());
                frames.push(frame);
            }
        }
    }
    if !reports
        .iter()
        .any(|report| report.action == "layered_dispatch")
    {
        cursor.dispatch = None;
    }
    for report in reports {
        if let Some(frame) = super::v4::outbox::frame(worker, &report, &mut cursor.dispatch).await?
        {
            frames.push(frame);
        }
        cursor.root = Some(report.key);
    }
    Ok(frames)
}

pub(crate) async fn empty(worker: &Worker) -> Result<bool> {
    if worker.inner.journal.lock().await.has_terminal_reports() {
        return Ok(false);
    }
    let empty = !worker.inner.state.store.has_pending_reports().await?;
    if empty {
        // The caller skips frames() when no reports remain. Release the last
        // parsed dispatch here too, after its final acknowledgement.
        worker.inner.report_cursor.lock().await.dispatch = None;
    }
    Ok(empty)
}

pub async fn ack(worker: &Worker, reference: &ExecutionRef, input: Value) -> Result<RpcReply> {
    let mut journal = worker.inner.journal.lock().await;
    let Some(last_sequence) = journal.terminal_ack_sequence(reference)? else {
        return Ok(RpcReply::error(404, "source execution missing"));
    };
    let sequence = input["sequence"].as_u64().unwrap_or(0);
    ensure!(sequence <= last_sequence, "invalid acknowledgement cursor");
    let ack = ReportAck {
        execution_id: reference.id.clone(),
        action: "layered_terminal".into(),
        item: String::new(),
        cursor: sequence,
    };
    save_ack(&worker.inner.state.store, &mut journal, &ack).await?;
    Ok(RpcReply::ok(json!({"acknowledged":sequence})))
}

async fn save_ack(store: &Arc<dyn Store>, journal: &mut Journal, ack: &ReportAck) -> Result<()> {
    let legacy = release::ReportStateCompatibility::current().writer == 0;
    save_ack_with_format(store, journal, ack, legacy).await
}

async fn save_ack_with_format(
    store: &Arc<dyn Store>,
    journal: &mut Journal,
    ack: &ReportAck,
    legacy: bool,
) -> Result<()> {
    // During the reader bridge, the old representation must become durable
    // first. A crash between writes is repaired by the legacy import at boot.
    if legacy {
        journal.apply_report_ack(ack, true)?;
    }
    store.report_ack(ack).await?;
    if !legacy {
        journal.apply_report_ack(ack, false)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "outbox/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/journal_memory/dispatch.rs"]
mod memory_tests;
