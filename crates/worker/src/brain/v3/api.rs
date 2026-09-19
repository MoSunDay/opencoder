use super::state;
use crate::Worker;
use anyhow::{ensure, Context, Result};
use opencoder_brain::scheduler;
use opencoder_core::{brain::*, fleet::*, message::now_ms};
use serde_json::{json, Value};

pub async fn handle(
    worker: &Worker,
    reference: &ExecutionRef,
    action: &str,
    input: Value,
) -> Result<RpcReply> {
    if matches!(action, "scheduler_output" | "scheduler_summary") {
        return super::output::query(worker, reference, action, input).await;
    }
    ensure!(
        reference.kind == ExecutionKind::Brain,
        "expected brain root"
    );
    let id = &reference.id;
    let gate = worker.lifecycle_gate(id).await;
    let _guard = gate.lock().await;
    let record = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(id)
        .context("root execution missing")?
        .clone();
    let request = state::request(&record)?;
    let snapshot = match worker.inner.state.store.brain_scheduler(id).await? {
        Some(snapshot) => snapshot,
        None => {
            worker
                .inner
                .state
                .store
                .commit_brain_scheduler(&scheduler::initialize(id, &request, now_ms())?)
                .await?
        }
    };
    let change = match action {
        "scheduler_intent" => {
            let intent: BrainDispatchIntent = serde_json::from_value(input.clone())?;
            ensure!(
                intent.generation == snapshot.run.generation + 1,
                "scheduler intent generation mismatch"
            );
            state::annotate(worker, id, "scheduler_intent", input).await?;
            return Ok(RpcReply::ok(json!({"stored":true})));
        }
        "scheduler_wake_ack" => {
            let generation = input["generation"]
                .as_u64()
                .context("generation required")?;
            if generation <= snapshot.run.generation {
                state::annotate(worker, id, "scheduler_wake_ack", json!(generation)).await?;
            }
            return Ok(RpcReply::ok(json!({"acknowledged":generation})));
        }
        "scheduler_dispatch_ack" => {
            let operation_id = input["operation_id"]
                .as_str()
                .context("operation_id required")?;
            let mut acks = record.annotations["scheduler_dispatch_acks"].clone();
            if !acks.is_object() {
                acks = json!({});
            }
            acks[operation_id] = json!(true);
            state::annotate(worker, id, "scheduler_dispatch_acks", acks).await?;
            return Ok(RpcReply::ok(json!({"acknowledged":operation_id})));
        }
        "snapshot" => return Ok(RpcReply::ok(json!(snapshot))),
        "events" => {
            let limit = input["limit"].as_u64().unwrap_or(100).clamp(1, 500) as u32;
            let events = worker
                .inner
                .state
                .store
                .brain_scheduler_events(id, input["after"].as_u64().unwrap_or(0), limit)
                .await?;
            return Ok(RpcReply::ok(
                json!({"more":events.len()==limit as usize,"next_seq":events.last().map(|e|e.seq),"events":events}),
            ));
        }
        "round" => {
            let round = input["round"].as_u64().context("round required")?;
            return Ok(RpcReply::ok(
                json!({"run_id":id,"round":round,"operations":snapshot.operations.iter().filter(|o| u64::from(o.round)==round).collect::<Vec<_>>()}),
            ));
        }
        "scheduler_context" => {
            let mut context: BrainSchedulerContext = serde_json::from_value(input)?;
            if snapshot.run.phase != BrainSchedulerPhase::Ready
                || context.generation != snapshot.run.generation
            {
                return Ok(RpcReply::ok(json!({"stale":true})));
            }
            ensure!(
                context.run_id == *id
                    && context.schema_version == 3
                    && context.request == request
                    && context.operations == snapshot.operations
                    && context.round == snapshot.run.round,
                "scheduler context identity mismatch"
            );
            let mut change = scheduler::change(&snapshot, now_ms());
            context.generation = change.run.generation;
            // Context is a finite root execution activation input, never an event.
            state::annotate(worker, id, "scheduler_context", json!(context)).await?;
            change.run.phase = BrainSchedulerPhase::Deciding;
            change
                .events
                .push(scheduler::event(&change.run, "decision_started", None));
            change
        }
        "scheduler_block" => {
            if input["generation"] != snapshot.run.generation
                || snapshot.run.phase != BrainSchedulerPhase::Ready
            {
                return Ok(RpcReply::ok(json!({"stale":true})));
            }
            scheduler::block(
                &snapshot,
                input["error"].as_str().context("error required")?.into(),
                now_ms(),
            )
        }
        "scheduler_authorize" => {
            let op: BrainOperation = serde_json::from_value(input["operation"].clone())?;
            let intent: BrainDispatchIntent =
                serde_json::from_value(record.annotations["scheduler_intent"].clone())?;
            let allowed = snapshot.run.phase == BrainSchedulerPhase::Waiting
                && snapshot.operations.iter().any(|o| {
                    o == &op && o.status == BrainOperationStatus::Creating && !o.cancel_requested
                })
                && intent.items.iter().any(|i| json!(i) == input["item"])
                && intent
                    .capabilities
                    .iter()
                    .any(|c| json!(c) == input["capability"]);
            return Ok(if allowed {
                RpcReply::ok(json!({"authorized":true}))
            } else {
                RpcReply::error(409, "dispatch fenced by scheduler state")
            });
        }
        "scheduler_receipt" => {
            let operation_id = input["operation_id"]
                .as_str()
                .context("operation_id required")?;
            let reply: RpcReply = serde_json::from_value(input["reply"].clone())?;
            let op = snapshot
                .operations
                .iter()
                .find(|op| op.operation_id == operation_id)
                .context("unknown operation")?;
            if op.status.terminal() || op.status == BrainOperationStatus::Running {
                return Ok(RpcReply::ok(json!({"duplicate":true})));
            }
            if reply.status >= 500 || matches!(reply.status, 408 | 423 | 429) {
                return Ok(RpcReply::ok(json!({"retry":true})));
            }
            if reply.status >= 300 {
                scheduler::terminal(
                    &snapshot,
                    &BrainSchedulerTerminalEvent {
                        run_id: id.clone(),
                        operation_id: op.operation_id.clone(),
                        execution_kind: op.execution_kind,
                        execution_id: op.execution_id.clone(),
                        status: BrainOperationStatus::Error,
                        source_sequence: 0,
                    },
                    now_ms(),
                )?
                .context("rejected dispatch already terminal")?
            } else {
                let mut change = scheduler::change(&snapshot, now_ms());
                change
                    .operations
                    .iter_mut()
                    .find(|o| o.operation_id == operation_id)
                    .unwrap()
                    .status = BrainOperationStatus::Running;
                let mut event = scheduler::event(&change.run, "operation_admitted", None);
                event.execution_id = Some(op.execution_id.clone());
                event.execution_kind = Some(op.execution_kind);
                event.capability_id = Some(op.capability_id.clone());
                change.events.push(event);
                change
            }
        }
        "scheduler_terminal" => {
            let notice = serde_json::from_value(input)?;
            let Some(change) = scheduler::terminal(&snapshot, &notice, now_ms())? else {
                return Ok(RpcReply::ok(json!({"duplicate":true})));
            };
            change
        }
        "scheduler_cancel_ack" => {
            let op: BrainOperation = serde_json::from_value(input)?;
            ensure!(
                snapshot
                    .operations
                    .iter()
                    .any(|o| o.operation_id == op.operation_id && o.cancel_requested),
                "unknown cancellation"
            );
            let mut acks = record.annotations["scheduler_cancel_acks"].clone();
            if !acks.is_object() {
                acks = json!({});
            }
            acks[&op.operation_id] = json!(true);
            state::annotate(worker, id, "scheduler_cancel_acks", acks).await?;
            return Ok(RpcReply::ok(json!({"acknowledged":op.operation_id})));
        }
        "pause" | "resume" | "cancel" => scheduler::command(&snapshot, action, now_ms())?,
        _ => return Ok(RpcReply::error(400, "unknown v3 scheduler operation")),
    };
    let next = worker
        .inner
        .state
        .store
        .commit_brain_scheduler(&change)
        .await?;
    state::settle(worker, &next).await?;
    Ok(RpcReply::ok(json!(next)))
}
