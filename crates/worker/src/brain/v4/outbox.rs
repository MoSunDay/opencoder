//! Only selected pending messages materialize dispatch parameters.
use super::state;
use crate::Worker;
use anyhow::{Context, Result};
use opencoder_core::{brain::layered::LayeredOperation, fleet::*};
use opencoder_store::reporting::{PendingReport, ReportAck};

pub async fn frame(
    worker: &Worker,
    report: &PendingReport,
    dispatch: &mut Option<crate::journal::DispatchCache>,
) -> Result<Option<NodeFrame>> {
    if report.action == "settle" {
        if !terminal(worker, &report.execution_id).await? {
            let snapshot = state::load(worker, &report.execution_id).await?;
            state::settle(worker, &snapshot).await?;
        }
        let terminal = terminal(worker, &report.execution_id).await?;
        if terminal {
            worker
                .inner
                .state
                .store
                .report_ack(&ReportAck {
                    execution_id: report.execution_id.clone(),
                    action: "settle".into(),
                    item: String::new(),
                    cursor: report.cursor,
                })
                .await?;
        }
        return Ok(None);
    }
    let input = if report.action == "layered_dispatch" {
        let operation: LayeredOperation = serde_json::from_value(report.payload.clone())?;
        worker
            .inner
            .journal
            .lock()
            .await
            .dispatch_payload(&report.execution_id, &operation, dispatch)
            .await?
    } else {
        report.payload.clone()
    };
    Ok(Some(NodeFrame::Brain {
        execution: ExecutionRef {
            id: report.execution_id.clone(),
            kind: ExecutionKind::Brain,
        },
        action: report.action.clone(),
        input,
    }))
}

async fn terminal(worker: &Worker, id: &str) -> Result<bool> {
    Ok(worker
        .inner
        .journal
        .lock()
        .await
        .summaries
        .get(id)
        .context("root execution missing")?
        .index
        .status
        .terminal())
}
