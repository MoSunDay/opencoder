//! Root-side v4 layered operations.
//!
//! Control owns catalog resolution, child creations and delivery; the node owns
//! the run phase, the generation fence, the operation indexes and the layer
//! barrier. Nothing here inspects a child body.
use super::{parent, state};
use crate::Worker;
use anyhow::{ensure, Context, Result};
use opencoder_brain::layered;
use opencoder_core::{
    brain::{layered::*, BrainCapabilityDescriptor},
    fleet::*,
    message::now_ms,
};
use serde_json::{json, Value};

pub async fn handle(
    worker: &Worker,
    reference: &ExecutionRef,
    action: &str,
    input: Value,
) -> Result<RpcReply> {
    if action.ends_with("_output") || action.ends_with("_summary") {
        return super::output::query(worker, reference, action, input).await;
    }
    ensure!(
        reference.kind == ExecutionKind::Brain,
        "expected brain root"
    );
    let id = &reference.id;
    let gate = worker.lifecycle_gate(id).await;
    let _guard = gate.lock().await;
    if matches!(
        action,
        "layered_wake_ack" | "layered_guidance_ack" | "layered_dispatch_ack" | "layered_cancel_ack"
    ) {
        return super::super::outbox::root_ack(worker, reference, action, input).await;
    }
    let record = worker
        .inner
        .journal
        .lock()
        .await
        .get(id)
        .await?
        .context("root execution missing")?;
    let request = state::request(&record)?;
    let snapshot = match worker.inner.state.store.brain_layered(id).await? {
        Some(snapshot) => snapshot,
        None => {
            worker
                .inner
                .state
                .store
                .commit_brain_layered(&layered::initialize(id, &request, now_ms())?)
                .await?
        }
    };
    let change = match action {
        "layered_intent" => {
            let intent: LayeredDispatchIntent = serde_json::from_value(input.clone())?;
            layered::validate_execution_io(
                opencoder_core::brain::execution_io_version(&record.assignment.request.input)?,
                &LayeredDecision::DispatchLayer {
                    layer: intent.layer,
                    reflection: None,
                    assignments: intent.assignments.clone(),
                    assessments: Default::default(),
                    reason: String::new(),
                    evidence_execution_ids: vec![],
                },
            )?;
            ensure!(
                intent.generation == snapshot.run.generation + 1,
                "layered intent generation mismatch"
            );
            state::annotate(worker, id, "layered_intent", input).await?;
            return Ok(RpcReply::ok(json!({"stored":true})));
        }
        "snapshot" => return Ok(RpcReply::ok(json!(snapshot))),
        "events" => {
            let limit = input["limit"].as_u64().unwrap_or(100).clamp(1, 500) as u32;
            let events = worker
                .inner
                .state
                .store
                .brain_layered_events(id, input["after"].as_u64().unwrap_or(0), limit)
                .await?;
            return Ok(RpcReply::ok(json!({
                "finished": snapshot.run.phase.terminal(),
                "more": events.len() == limit as usize,
                "next_seq": events.last().map(|event| event.seq),
                "events": events
            })));
        }
        "layer" | "round" => {
            let layer = input["layer"]
                .as_u64()
                .or(input["round"].as_u64())
                .context("layer required")?;
            let operations: Vec<&LayeredOperation> = snapshot
                .operations
                .iter()
                .filter(|op| u64::from(op.layer) == layer)
                .collect();
            let intent = record.annotations["layered_intent"].clone();
            let assignments = if intent["layer"] == layer {
                intent["assignments"].clone()
            } else {
                json!([])
            };
            return Ok(RpcReply::ok(
                json!({"run_id":id,"layer":layer,"phase":snapshot.run.phase,
                    "operations":operations,"assignments":assignments}),
            ));
        }
        "layered_context" => {
            let mut context: LayeredContext = serde_json::from_value(input)?;
            if context.execution_io_version
                != opencoder_core::brain::execution_io_version(&record.assignment.request.input)?
                || (context.execution_io_version == 0 && !context.execution_artifacts.is_empty())
            {
                return Ok(RpcReply::error(
                    409,
                    "layered context execution IO version differs from frozen admission",
                ));
            }
            if snapshot.run.phase != LayeredPhase::Ready
                || context.generation != snapshot.run.generation
            {
                return Ok(RpcReply::ok(json!({"stale":true})));
            }
            ensure!(
                context.run_id == *id
                    && context.schema_version == 7
                    && context.request == request
                    && context.operations == layered::relevant_operations(&snapshot)
                    && context.guidance_only != layered::barrier(&snapshot)
                    && context.layer == snapshot.run.layer
                    && context.run.as_ref() == Some(&snapshot.run),
                "layered context identity mismatch"
            );
            if !context.execution_artifacts.iter().all(|(id, files)| {
                context.operations.iter().any(|operation| {
                    &operation.execution_id == id
                        && operation.status.terminal()
                        && files.iter().all(|file| {
                            file.execution.id == *id
                                && file.execution.kind == operation.execution_kind
                        })
                })
            }) {
                return Ok(RpcReply::error(
                    409,
                    "artifact index does not belong to this run's terminal execution",
                ));
            }
            let mut change = layered::change(&snapshot, now_ms());
            context.generation = change.run.generation;
            if let Err(error) = super::budget::context(worker, &record, &context) {
                let next = worker
                    .inner
                    .state
                    .store
                    .commit_brain_layered(&layered::block(&snapshot, error.to_string(), now_ms()))
                    .await?;
                state::settle(worker, &next).await?;
                return Ok(RpcReply::ok(json!(next)));
            }
            // Context is a finite root activation input, never an event.
            state::annotate(worker, id, "layered_context", json!(context)).await?;
            change.run.phase = LayeredPhase::Deciding;
            change
                .events
                .push(layered::event(&change.run, "decision_started", None));
            change
        }
        "human_input" => {
            ensure!(!snapshot.run.phase.terminal(), "run is terminal");
            let text = input["text"].as_str().context("text required")?.trim();
            ensure!(
                !text.is_empty() && text.len() <= 4096,
                "text must contain 1..4096 bytes"
            );
            if let Err(error) = super::budget::human(worker, &record, text).await {
                return Ok(RpcReply::error(413, error.to_string()));
            }
            let mut change = layered::change(&snapshot, now_ms());
            change.run.pending_guidance = true;
            let mut event = layered::event(&change.run, "human_input", None);
            event.user_input = Some(text.to_owned());
            change.events.push(event);
            if matches!(
                snapshot.run.phase,
                LayeredPhase::Deciding | LayeredPhase::Waiting | LayeredPhase::Blocked
            ) {
                change.run.phase = LayeredPhase::Ready;
                if snapshot.run.phase == LayeredPhase::Deciding {
                    state::annotate(worker, id, "layered_context", Value::Null).await?;
                }
            }
            change
        }
        "set_round_budget" => {
            ensure!(
                matches!(
                    snapshot.run.phase,
                    LayeredPhase::Blocked | LayeredPhase::Paused
                ),
                "pause or block before adjusting budget"
            );
            let budget = input["max_rounds"]
                .as_u64()
                .context("max_rounds required")?;
            ensure!(
                budget > u64::from(snapshot.run.round) && budget <= 32,
                "budget must exceed current round and be at most 32"
            );
            let mut change = layered::change(&snapshot, now_ms());
            change.run.max_rounds = budget as u32;
            change.events.push(layered::event(
                &change.run,
                "round_budget_changed",
                Some(format!("budget set to {budget}")),
            ));
            change
        }
        "layered_block" => {
            if input["generation"] != snapshot.run.generation
                || snapshot.run.phase != LayeredPhase::Ready
            {
                return Ok(RpcReply::ok(json!({"stale":true})));
            }
            layered::block(
                &snapshot,
                input["error"].as_str().context("error required")?.into(),
                now_ms(),
            )
        }
        "layered_authorize" => {
            let op: LayeredOperation = serde_json::from_value(input["operation"].clone())?;
            let assignment = input
                .get("assignment")
                .or_else(|| input.get("item"))
                .context("assignment required")?;
            let capability: BrainCapabilityDescriptor =
                serde_json::from_value(input["capability"].clone())?;
            let intent: LayeredDispatchIntent =
                serde_json::from_value(record.annotations["layered_intent"].clone())?;
            // The durable intent outlives every admission and retry of its own
            // layer, so it only fences the layer it was decided for: its
            // generation must never be newer than the committed projection.
            let allowed = (snapshot.run.phase == LayeredPhase::Waiting
                || (snapshot.run.pending_guidance
                    && matches!(
                        snapshot.run.phase,
                        LayeredPhase::Ready | LayeredPhase::Deciding
                    )))
                && op.activation == snapshot.run.activation
                && op.capability_id == capability.capability_id
                && assignment["node_id"] == op.node_id
                && assignment["capability_id"] == op.capability_id
                && intent.layer == snapshot.run.layer
                && intent.generation <= snapshot.run.generation
                && snapshot.operations.iter().any(|current| {
                    current == &op
                        && current.status == LayeredOperationStatus::Creating
                        && !current.cancel_requested
                })
                && intent
                    .assignments
                    .iter()
                    .any(|item| json!(item) == *assignment)
                && intent
                    .capabilities
                    .iter()
                    .any(|item| json!(item) == json!(capability));
            return Ok(if allowed {
                RpcReply::ok(json!({"authorized":true}))
            } else {
                RpcReply::error(409, "dispatch fenced by layered state")
            });
        }
        "layered_receipt" => {
            let operation_id = input["operation_id"]
                .as_str()
                .context("operation_id required")?;
            let reply: RpcReply = serde_json::from_value(input["reply"].clone())?;
            let op = snapshot
                .operations
                .iter()
                .find(|op| op.operation_id == operation_id)
                .context("unknown operation")?;
            if op.status.terminal() || op.status == LayeredOperationStatus::Running {
                return Ok(RpcReply::ok(json!({"duplicate":true})));
            }
            if reply.status >= 500 || matches!(reply.status, 408 | 423 | 429) {
                return Ok(RpcReply::ok(json!({"retry":true})));
            }
            if reply.status >= 300 {
                let mut change = layered::terminal(
                    &snapshot,
                    &request,
                    &LayeredTerminalEvent {
                        run_id: id.clone(),
                        operation_id: op.operation_id.clone(),
                        execution_kind: op.execution_kind,
                        execution_id: op.execution_id.clone(),
                        status: LayeredOperationStatus::Error,
                        source_sequence: 0,
                    },
                    now_ms(),
                )?
                .context("rejected dispatch already terminal")?;
                let reason = format!(
                    "Execution admission rejected ({}): {}",
                    reply.status, reply.body
                );
                if let Some(event) = change
                    .events
                    .iter_mut()
                    .find(|event| event.event_type == "operation_terminal")
                {
                    event.reason_summary = Some(reason.chars().take(1024).collect());
                }
                change
            } else {
                let mut change = layered::change(&snapshot, now_ms());
                if snapshot.run.phase == LayeredPhase::Deciding && snapshot.run.pending_guidance {
                    // Admission changes the finite guidance context; wake a fresh one.
                    change.run.phase = LayeredPhase::Ready;
                }
                change
                    .operations
                    .iter_mut()
                    .find(|current| current.operation_id == operation_id)
                    .unwrap()
                    .status = LayeredOperationStatus::Running;
                let mut event = layered::event(&change.run, "operation_admitted", None);
                event.execution_id = Some(op.execution_id.clone());
                event.execution_kind = Some(op.execution_kind);
                event.capability_id = Some(op.capability_id.clone());
                change.events.push(event);
                change
            }
        }
        "layered_terminal" => {
            let notice = parent::notice(&snapshot, &input, id)?;
            let Some(change) = layered::terminal(&snapshot, &request, &notice, now_ms())? else {
                return Ok(RpcReply::ok(json!({"duplicate":true})));
            };
            change
        }
        "pause" | "resume" | "cancel" => {
            if action == "resume" {
                if let Err(error) = super::budget::human(worker, &record, "").await {
                    return Ok(RpcReply::error(413, error.to_string()));
                }
            }
            layered::command(&snapshot, &request.plan, action, now_ms())?
        }
        _ => return Ok(RpcReply::error(400, "unknown v4 layered operation")),
    };
    let next = worker
        .inner
        .state
        .store
        .commit_brain_layered(&change)
        .await?;
    // Notify committed transitions, never projection reads performed while
    // collecting an outbox report (which would trigger another report).
    opencoder_session::loop_registry::notify_change();
    state::settle(worker, &next).await?;
    if action == "human_input" && snapshot.run.phase == LayeredPhase::Deciding {
        if let Some(active) = worker.inner.active.lock().await.get(id).cloned() {
            active.cancel();
        }
    }
    if action == "layered_context" && next.run.phase == LayeredPhase::Deciding {
        // The root is normally idle after emitting its wake. Installing the
        // context only changes the durable projection; enqueue a fresh
        // activation so the node-local model runner consumes it. Wait for a
        // racing initial activation to finish before changing the journal
        // status back to Pending.
        drop(_guard);
        requeue_decision(worker, id).await?;
    }
    Ok(RpcReply::ok(json!(next)))
}

async fn requeue_decision(worker: &Worker, id: &str) -> Result<()> {
    loop {
        if worker.inner.active.lock().await.contains_key(id) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            continue;
        }
        let gate = worker.lifecycle_gate(id).await;
        let _guard = gate.lock().await;
        if worker.inner.active.lock().await.contains_key(id) {
            continue;
        }
        let record = worker
            .inner
            .journal
            .lock()
            .await
            .get(id)
            .await?
            .context("root execution missing while queueing layered decision")?;
        if !matches!(
            record.assignment.index.status,
            ExecutionStatus::Idle | ExecutionStatus::Interrupted
        ) {
            return Ok(());
        }
        let config = record
            .queue
            .as_ref()
            .map(|queued| queued.config.clone())
            .unwrap_or(worker.configuration()?);
        crate::operations::queue::enqueue(worker, record, config, true).await?;
        return Ok(());
    }
}
