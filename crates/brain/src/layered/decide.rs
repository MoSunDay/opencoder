//! One-layer decisions: the model may only dispatch the next layer, never
//! invent nodes, change bindings or skip a layer.
use super::{change, event, levels, terminal, validate};
use anyhow::{ensure, Context, Result};
use opencoder_core::brain::{layered::*, BrainCapabilityDescriptor};
use opencoder_core::fleet::ExecutionKind;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Deterministic child id: attempt is part of the identity, so a superseded
/// attempt can never collide with the live one.
pub fn execution_id(
    run: &str,
    layer: u32,
    node: &str,
    attempt: u32,
    kind: ExecutionKind,
) -> String {
    let digest = Sha256::digest(format!("{run}\0{layer}\0{node}\0{attempt}").as_bytes());
    format!("{}-{:x}", kind.prefix(), digest)[..kind.prefix().len() + 41].into()
}

pub fn operation_id(run: &str, layer: u32, node: &str, attempt: u32) -> String {
    format!("{run}#l{layer}#{node}#a{attempt}")
}

pub fn decide(
    snapshot: &LayeredSnapshot,
    request: &LayeredRequest,
    catalog: &[BrainCapabilityDescriptor],
    decision: &LayeredDecision,
    now: i64,
) -> Result<LayeredChange> {
    ensure!(
        snapshot.run.phase == LayeredPhase::Deciding,
        "run is not deciding"
    );
    ensure!(snapshot.run.layer <= 32, "invalid layer index");
    ensure!(
        terminal::layers_complete(&request.plan, &snapshot.operations, snapshot.run.layer)?,
        "decision requires a completed layer barrier"
    );
    let levels = levels::layers(&request.plan)?;
    let mut update = change(snapshot, now);
    match decision {
        LayeredDecision::DispatchLayer {
            layer,
            assignments,
            reason,
            evidence_execution_ids,
        } => {
            validate::reason(reason)?;
            validate::evidence(evidence_execution_ids, &snapshot.operations)?;
            ensure!(
                *layer == snapshot.run.layer + 1,
                "decision layer must be the next layer"
            );
            ensure!(
                *layer <= request.plan.max_rounds,
                "maximum scheduler layers exceeded"
            );
            let wanted = levels
                .get(*layer as usize - 1)
                .context("dispatch layer is out of the plan")?;
            ensure!(
                assignments.len() == wanted.len(),
                "layer dispatch must cover exactly {wanted_len} nodes",
                wanted_len = wanted.len()
            );
            let mut seen = BTreeSet::new();
            for assignment in assignments {
                ensure!(
                    seen.insert(assignment.node_id.as_str()),
                    "duplicate node assignment"
                );
                ensure!(
                    wanted.contains(&assignment.node_id),
                    "assignment is not part of the dispatched layer"
                );
            }
            update.run.layer = *layer;
            update.run.phase = LayeredPhase::Waiting;
            let mut started = event(&update.run, "layer_started", Some(reason.clone()));
            started.decision_summary = Some("dispatch_layer".into());
            started.evidence_execution_ids = evidence_execution_ids.clone();
            update.events.push(started);
            for assignment in assignments.iter().filter(|a| wanted.contains(&a.node_id)) {
                let node = request
                    .plan
                    .node(&assignment.node_id)
                    .context("unknown node")?;
                let descriptor = catalog
                    .iter()
                    .find(|c| c.capability_id == node.capability_id)
                    .context("illegal capability")?;
                ensure!(
                    descriptor.capability_id == node.capability_id,
                    "node binding is fixed at save time"
                );
                validate::bindings(
                    assignment,
                    descriptor,
                    request,
                    &request.plan,
                    &snapshot.operations,
                )?;
                let operation = LayeredOperation {
                    operation_id: operation_id(&snapshot.run.run_id, *layer, &node.node_id, 1),
                    run_id: snapshot.run.run_id.clone(),
                    layer: *layer,
                    node_id: node.node_id.clone(),
                    attempt: 1,
                    capability_id: descriptor.capability_id.clone(),
                    execution_kind: descriptor.kind,
                    execution_id: execution_id(
                        &snapshot.run.run_id,
                        *layer,
                        &node.node_id,
                        1,
                        descriptor.kind,
                    ),
                    status: LayeredOperationStatus::Creating,
                    source_sequence: None,
                    cancel_requested: false,
                };
                let mut dispatched = event(&update.run, "node_dispatched", None);
                dispatched.node_id = Some(node.node_id.clone());
                dispatched.attempt = Some(1);
                dispatched.capability_id = Some(descriptor.capability_id.clone());
                dispatched.execution_kind = Some(descriptor.kind);
                dispatched.execution_id = Some(operation.execution_id.clone());
                update.events.push(dispatched);
                update.operations.push(operation);
            }
        }
        LayeredDecision::Complete {
            reason,
            evidence_execution_ids,
            summary,
        } => {
            validate::reason(reason)?;
            validate::evidence(evidence_execution_ids, &snapshot.operations)?;
            ensure!(
                snapshot.run.layer as usize == levels.len(),
                "every layer must finish before completion"
            );
            ensure!(summary.chars().count() <= 16384, "summary is too large");
            update.run.phase = LayeredPhase::Completed;
            update.run.summary = Some(summary.clone());
            let mut done = event(&update.run, "run_completed", Some(reason.clone()));
            done.decision_summary = Some("complete".into());
            done.evidence_execution_ids = evidence_execution_ids.clone();
            update.events.push(done);
        }
        LayeredDecision::Fail { reason, error_type } => {
            validate::reason(reason)?;
            validate::reason(error_type)?;
            update.run.phase = LayeredPhase::Failed;
            update.run.error = Some(format!("{error_type}: {reason}"));
            update
                .events
                .push(event(&update.run, "run_failed", Some(reason.clone())));
            terminal::cancel_pending(&mut update);
        }
    }
    Ok(update)
}
