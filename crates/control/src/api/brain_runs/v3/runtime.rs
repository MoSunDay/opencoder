use super::gateway;
use crate::{api::brain_runs::catalog, AppState};
use anyhow::Result;
use opencoder_core::brain::*;
use opencoder_core::fleet::{ExecutionCommand, NodeOperation};
use serde_json::Value;
use std::sync::Arc;

pub async fn wake(state: &Arc<AppState>, run_id: &str) -> Result<()> {
    let Some(snapshot) = state.store.brain_scheduler(run_id).await? else {
        return Ok(());
    };
    if !matches!(
        snapshot.run.phase,
        BrainSchedulerPhase::Ready | BrainSchedulerPhase::Deciding
    ) {
        return Ok(());
    }
    let Some(assignment) = state.fleet.assignment(run_id).await? else {
        return Ok(());
    };
    let Some(value) = assignment.request.input.get("scheduler_request") else {
        return Ok(());
    };
    let request: BrainSchedulerRequest = serde_json::from_value(value.clone())?;
    let raw = catalog::capabilities(state).await?;
    let mut all = Vec::new();
    for value in raw {
        let Some(id) = value
            .get("id")
            .or_else(|| value.get("capability_id"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Ok(kind) = serde_json::from_value(value.get("kind").cloned().unwrap_or(Value::Null))
        else {
            continue;
        };
        all.push(BrainCapabilityDescriptor {
            capability_id: id.into(),
            kind,
            target: value
                .get("target")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            input_desc: value
                .get("input_desc")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            output_desc: value
                .get("output_desc")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            required_inputs: value
                .get("required_inputs")
                .and_then(Value::as_array)
                .map(|v| {
                    v.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            definition: value.get("definition").cloned().unwrap_or(Value::Null),
            version: value
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("current")
                .into(),
        });
    }
    let catalog = opencoder_brain::scheduler::prefilter(&all, &request);
    let snapshot = if snapshot.run.phase == BrainSchedulerPhase::Ready {
        let mut deciding =
            opencoder_brain::scheduler::change(&snapshot, opencoder_core::message::now_ms());
        deciding.run.phase = BrainSchedulerPhase::Deciding;
        deciding.events.push(opencoder_brain::scheduler::event(
            &deciding.run,
            "decision_started",
            None,
        ));
        match state.store.commit_brain_scheduler(&deciding).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let _ = state
                    .store
                    .commit_brain_scheduler(&opencoder_brain::scheduler::block(
                        &snapshot,
                        error.to_string(),
                        opencoder_core::message::now_ms(),
                    ))
                    .await;
                return Ok(());
            }
        }
    } else {
        snapshot
    };
    let summaries = execution_summaries(state, &snapshot.operations).await?;
    let context = BrainSchedulerContext {
        schema_version: 3,
        run_id: run_id.into(),
        generation: snapshot.run.generation,
        round: snapshot.run.round,
        request: request.clone(),
        capabilities: catalog.clone(),
        operations: snapshot.operations.clone(),
        summaries,
    };
    let decision = match state.brain.scheduler_decide(&context).await {
        Ok(decision) => decision,
        Err(error) => {
            let change = opencoder_brain::scheduler::block(
                &snapshot,
                error.to_string(),
                opencoder_core::message::now_ms(),
            );
            let _ = state.store.commit_brain_scheduler(&change).await;
            return Ok(());
        }
    };
    let change = match opencoder_brain::scheduler::decide(
        &snapshot,
        &request,
        &catalog,
        &decision,
        opencoder_core::message::now_ms(),
    ) {
        Ok(change) => change,
        Err(error) => {
            let change = opencoder_brain::scheduler::block(
                &snapshot,
                error.to_string(),
                opencoder_core::message::now_ms(),
            );
            let _ = state.store.commit_brain_scheduler(&change).await;
            return Ok(());
        }
    };
    if let BrainSchedulerDecision::Dispatch { capabilities, .. } = &decision {
        let intent = BrainDispatchIntent {
            generation: change.run.generation,
            operations: change
                .operations
                .iter()
                .filter(|operation| operation.round == change.run.round)
                .cloned()
                .collect(),
            items: capabilities.clone(),
            capabilities: catalog.clone(),
        };
        if let Some(index) = state.fleet.index(run_id).await? {
            let reply = state
                .hub
                .call(
                    &index.node_id,
                    NodeOperation::Brain {
                        execution: index.execution_ref(),
                        action: "scheduler_intent".into(),
                        input: serde_json::to_value(&intent)?,
                    },
                )
                .await;
            anyhow::ensure!(
                reply.status < 300,
                "persist scheduler intent: {}",
                reply.body
            );
        }
    }
    let next = state.store.commit_brain_scheduler(&change).await?;
    for op in next.operations.iter().filter(|op| op.cancel_requested) {
        if let Some(index) = state.fleet.index(&op.execution_id).await? {
            let _ = state
                .hub
                .call(
                    &index.node_id,
                    opencoder_core::fleet::NodeOperation::Command {
                        execution: index.execution_ref(),
                        command: opencoder_core::fleet::ExecutionCommand {
                            action: "cancel".into(),
                            input: Value::Null,
                        },
                    },
                )
                .await;
        }
    }
    for op in next
        .operations
        .iter()
        .filter(|op| op.round == next.run.round && op.status == BrainOperationStatus::Creating)
    {
        let item = match &decision {
            BrainSchedulerDecision::Dispatch { capabilities, .. } => capabilities
                .iter()
                .find(|x| x.capability_id == op.capability_id),
            _ => None,
        };
        let Some(item) = item else {
            continue;
        };
        let Some(cap) = catalog.iter().find(|x| x.capability_id == op.capability_id) else {
            continue;
        };
        dispatch_operation(state, &next.run, op, item, cap).await?;
    }
    Ok(())
}

pub(crate) async fn dispatch_operation(
    state: &Arc<AppState>,
    run: &BrainSchedulerRun,
    operation: &BrainOperation,
    item: &BrainDispatchItem,
    capability: &BrainCapabilityDescriptor,
) -> Result<bool> {
    let Some(snapshot) = state.store.brain_scheduler(&run.run_id).await? else {
        return Ok(true);
    };
    if snapshot.run.phase != BrainSchedulerPhase::Waiting
        || snapshot
            .operations
            .iter()
            .find(|candidate| candidate.operation_id == operation.operation_id)
            .is_none_or(|candidate| candidate.status != BrainOperationStatus::Creating)
    {
        return Ok(true);
    }
    let reply = gateway::dispatch(state, run, operation, item, capability).await?;
    if reply.status >= 500 || matches!(reply.status, 408 | 423 | 429) {
        // Admission can be transient (no ready node, drain, or a transport
        // timeout). Keep the operation Creating so the durable outbox retries
        // it after the next heartbeat or restart.
        return Ok(false);
    }
    if reply.status >= 300 {
        let terminal = BrainSchedulerTerminalEvent {
            run_id: run.run_id.clone(),
            operation_id: operation.operation_id.clone(),
            execution_kind: operation.execution_kind,
            execution_id: operation.execution_id.clone(),
            status: BrainOperationStatus::Error,
            source_sequence: 0,
        };
        apply_terminal(state, terminal).await?;
    } else if let Some(current) = state.store.brain_scheduler(&run.run_id).await? {
        if let Ok(admitted) = opencoder_brain::scheduler::admit(
            &current,
            &operation.operation_id,
            opencoder_core::message::now_ms(),
        ) {
            let _ = state.store.commit_brain_scheduler(&admitted).await;
        }
    }
    Ok(true)
}

async fn execution_summaries(
    state: &Arc<AppState>,
    operations: &[BrainOperation],
) -> Result<std::collections::BTreeMap<String, String>> {
    let mut summaries = std::collections::BTreeMap::new();
    for operation in operations
        .iter()
        .filter(|operation| operation.status.successful())
    {
        let Some(index) = state.fleet.index(&operation.execution_id).await? else {
            continue;
        };
        let reply = state
            .hub
            .call(
                &index.node_id,
                NodeOperation::Inspect {
                    execution: index.execution_ref(),
                },
            )
            .await;
        if reply.status < 300 {
            let text = serde_json::to_string(&reply.body["result"])
                .unwrap_or_default()
                .chars()
                .take(4096)
                .collect::<String>();
            summaries.insert(operation.execution_id.clone(), text);
        }
    }
    Ok(summaries)
}

pub async fn apply_terminal(
    state: &Arc<AppState>,
    notice: BrainSchedulerTerminalEvent,
) -> Result<()> {
    let Some(snapshot) = state.store.brain_scheduler(&notice.run_id).await? else {
        return Ok(());
    };
    let Some(change) = opencoder_brain::scheduler::terminal(
        &snapshot,
        &notice,
        opencoder_core::message::now_ms(),
    )?
    else {
        return Ok(());
    };
    let next = state.store.commit_brain_scheduler(&change).await?;
    request_cancellations(state, &next).await?;
    if next.run.phase == BrainSchedulerPhase::Ready {
        Box::pin(wake(state, &notice.run_id)).await?;
    }
    Ok(())
}

pub(crate) async fn request_cancellations(
    state: &Arc<AppState>,
    snapshot: &BrainSchedulerSnapshot,
) -> Result<()> {
    for operation in snapshot
        .operations
        .iter()
        .filter(|operation| operation.cancel_requested && !operation.status.terminal())
    {
        if let Some(index) = state.fleet.index(&operation.execution_id).await? {
            let _ = state
                .hub
                .call(
                    &index.node_id,
                    NodeOperation::Command {
                        execution: index.execution_ref(),
                        command: ExecutionCommand {
                            action: "cancel".into(),
                            input: Value::Null,
                        },
                    },
                )
                .await;
        }
    }
    Ok(())
}
