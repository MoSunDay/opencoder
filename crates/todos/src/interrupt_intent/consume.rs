//! Driver-side consumption of the intent channel: the one place a request
//! becomes a database write, and only on the writer the live `run` /
//! `resume` process already holds.

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use opencoder_store::Store;

use super::{expired, mark, read, remove, ConsumeOutcome, Decision, IntentStatus, InterruptIntent};
use crate::{persistence, transitions, types::*};

/// Maximum generation-conflict retries for the driver's commit. The driving
/// loop bumps the generation on every transition, so a commit taken while a
/// batch is landing can lose the CAS race; reloading and retrying re-derives
/// the terminal-state check from the freshest state.
const INTERRUPT_COMMIT_RETRIES: u32 = 3;

/// Driver-side consumption of one pending intent.
///
/// * `requested` + not yet durable + workflow still at the request's
///   generation → commit a Suspended transition carrying `request_id`, then
///   mark the file applied.
/// * `requested` + already in the event log → mark applied without a second
///   commit (crash between commit and the applied rewrite).
/// * `requested` + workflow past the request's generation → mark stale: a
///   leftover request from an earlier run never suspends a later one.
/// * `requested` + terminal workflow → mark terminal (the publisher reports
///   it instead of waiting); unknown workflow → remove, nobody can apply it.
/// * any other status → a leftover acknowledgement; removed once expired.
pub async fn consume_pending(
    store: &Arc<dyn Store>,
    root: &Path,
    workflow_id: &str,
) -> Result<ConsumeOutcome> {
    let Some(intent) = read(root, workflow_id).await? else {
        return Ok(ConsumeOutcome::None);
    };
    if intent.status != IntentStatus::Requested {
        if expired(&intent, opencoder_core::message::now_ms()) {
            remove(root, workflow_id).await?;
        }
        return Ok(ConsumeOutcome::None);
    }
    let Some((_, state)) = persistence::load(store, workflow_id).await? else {
        remove(root, workflow_id).await?;
        return Ok(ConsumeOutcome::None);
    };
    if matches!(
        state.status,
        WorkflowStatus::Completed | WorkflowStatus::Failed
    ) {
        decide(root, &intent, IntentStatus::Terminal, &state).await?;
        return Ok(ConsumeOutcome::Terminal(state));
    }
    if already_applied(store, &intent).await? {
        decide(root, &intent, IntentStatus::Applied, &state).await?;
        return Ok(ConsumeOutcome::AlreadyApplied(state));
    }
    if state.generation != intent.generation {
        tracing::info!(
            workflow_id = %workflow_id,
            request_id = %intent.request_id,
            request_generation = intent.generation,
            generation = state.generation,
            "stale todo interrupt intent dropped; a later run is not cancelled by it"
        );
        decide(root, &intent, IntentStatus::Stale, &state).await?;
        return Ok(ConsumeOutcome::Stale(state));
    }
    match commit_interrupt(store, workflow_id, &intent.reason, Some(&intent.request_id)).await {
        Ok(state) => {
            decide(root, &intent, IntentStatus::Applied, &state).await?;
            Ok(ConsumeOutcome::Applied(state))
        }
        Err(error) => Err(error),
    }
}

/// Record the driver's decision in the file the publisher polls. A file that
/// was replaced by a newer request meanwhile is left to that request.
async fn decide(
    root: &Path,
    intent: &InterruptIntent,
    status: IntentStatus,
    state: &WorkflowState,
) -> Result<()> {
    mark(
        root,
        &intent.workflow_id,
        &intent.request_id,
        status,
        Decision {
            generation: state.generation,
            at_ms: opencoder_core::message::now_ms(),
        },
    )
    .await?;
    Ok(())
}

/// Commit a Suspended transition for `workflow_id`. `request_id` (when present)
/// is recorded in the event payload for idempotent intent consumption. This is
/// also the implementation of the public `interrupt` entrypoint.
pub(crate) async fn commit_interrupt(
    store: &Arc<dyn Store>,
    workflow_id: &str,
    reason: &str,
    request_id: Option<&str>,
) -> Result<WorkflowState> {
    let mut retries_left = INTERRUPT_COMMIT_RETRIES;
    loop {
        let (spec, state) = persistence::load(store, workflow_id)
            .await?
            .with_context(|| format!("todo workflow not found: {workflow_id}"))?;
        if matches!(
            state.status,
            WorkflowStatus::Completed | WorkflowStatus::Failed
        ) {
            anyhow::bail!("cannot interrupt terminal workflow {workflow_id}");
        }
        let next = transitions::terminal(state, WorkflowStatus::Suspended, reason.to_owned())?;
        let mut payload = serde_json::json!({"reason": reason});
        if let Some(request_id) = request_id {
            payload["request_id"] = serde_json::json!(request_id);
        }
        match persistence::commit(store, &spec, &next, "workflow_interrupted", payload).await {
            Ok(_) => {
                tracing::info!(
                    workflow_id = %workflow_id,
                    reason = %reason,
                    request_id = request_id.unwrap_or("-"),
                    "todo workflow interrupted"
                );
                return Ok(next);
            }
            Err(error)
                if retries_left > 0 && format!("{error:#}").contains("generation conflict") =>
            {
                retries_left -= 1;
                tracing::info!(
                    workflow_id = %workflow_id,
                    retries_left,
                    "interrupt commit lost the generation race; reloading and retrying"
                );
            }
            Err(error) => return Err(error),
        }
    }
}

/// Whether `intent`'s request id is already in the workflow's event log: the
/// commit happened, only the applied rewrite was lost.
pub(crate) async fn already_applied(
    store: &Arc<dyn Store>,
    intent: &InterruptIntent,
) -> Result<bool> {
    let events = store.todo_events_after(&intent.workflow_id, 0).await?;
    Ok(events.iter().any(|event| {
        event.kind == "workflow_interrupted"
            && event.payload["request_id"].as_str() == Some(intent.request_id.as_str())
    }))
}
