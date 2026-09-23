use super::{change, event, terminal, validate};
use anyhow::{ensure, Context, Result};
use opencoder_core::{
    brain::{layered::*, BrainCapabilityDescriptor},
    fleet::ExecutionKind,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

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
    ensure!(
        terminal::barrier(snapshot),
        "all dispatched executions must terminate before deciding"
    );
    let groups = super::layers(&request.plan)?;
    let mut update = change(snapshot, now);
    match decision {
        LayeredDecision::DispatchLayer {
            layer,
            assignments,
            reason,
            evidence_execution_ids,
            reflection,
            assessments,
        } => {
            validate::reason(reason)?;
            validate::evidence(evidence_execution_ids, &snapshot.operations)?;
            let target = layer
                .checked_sub(1)
                .and_then(|i| groups.get(i as usize))
                .context("unknown target layer")?;
            let back = *layer <= snapshot.run.layer;
            assess(&mut update, snapshot, request, assessments, !back)?;
            if back {
                let reflection = reflection
                    .as_ref()
                    .context("return requires a reflection")?;
                validate::reason(reflection)?;
                ensure!(
                    request.plan.edges.iter().any(|e| request
                        .plan
                        .node(&e.from)
                        .is_some_and(|n| n.layer == snapshot.run.layer)
                        && request.plan.node(&e.to).is_some_and(|n| n.layer == *layer)),
                    "return path is not configured"
                );
                if snapshot.run.round >= snapshot.run.max_rounds {
                    let mut blocked = super::block(
                        snapshot,
                        "round budget exhausted; increase budget and resume".into(),
                        now,
                    );
                    blocked.run.reflection = Some(reflection.clone());
                    blocked.events[0].reflection = Some(reflection.clone());
                    blocked.events.splice(0..0, update.events);
                    return Ok(blocked);
                }
                update.run.round += 1;
                update.run.valid_layers = layer - 1;
                update.run.reflection = Some(reflection.clone());
            } else {
                ensure!(
                    *layer == snapshot.run.layer + 1,
                    "forward transitions cannot skip layers"
                );
                ensure!(
                    terminal::current_successful(snapshot),
                    "failed layer must reflect or block"
                );
                ensure!(
                    reflection.is_none(),
                    "forward dispatch cannot declare a reflection"
                );
                update.run.valid_layers = snapshot.run.layer;
            }
            ensure!(
                !assignments.is_empty() && assignments.len() <= 256,
                "dispatch requires 1..256 capability executions"
            );
            let mut seen = BTreeSet::new();
            let mut covered = BTreeSet::new();
            for assignment in assignments {
                ensure!(
                    target.contains(&assignment.node_id),
                    "assignment outside target layer"
                );
                ensure!(
                    seen.insert((&assignment.node_id, &assignment.capability_id)),
                    "duplicate node capability assignment"
                );
                covered.insert(&assignment.node_id);
                let node = request
                    .plan
                    .node(&assignment.node_id)
                    .context("unknown milestone")?;
                ensure!(
                    node.capability_ids.contains(&assignment.capability_id),
                    "capability not attached to milestone"
                );
                let cap = catalog
                    .iter()
                    .find(|c| c.capability_id == assignment.capability_id)
                    .context("capability unavailable")?;
                validate::reason(&assignment.reason)?;
                validate::bindings(
                    assignment,
                    cap,
                    request,
                    &request.plan,
                    &snapshot.operations,
                )?;
            }
            ensure!(
                target.iter().all(|id| covered.contains(id)),
                "every milestone must dispatch at least one capability"
            );
            update.run.layer = *layer;
            update.run.activation += 1;
            update.run.phase = LayeredPhase::Waiting;
            update.run.error = None;
            let mut started = event(&update.run, "layer_started", Some(reason.clone()));
            started.decision_summary = Some(
                if back {
                    "reflect_and_return"
                } else {
                    "dispatch_layer"
                }
                .into(),
            );
            started.assignments = assignments.clone();
            started.reflection = update.run.reflection.clone();
            started.evidence_execution_ids = evidence_execution_ids.clone();
            update.events.push(started);
            for assignment in assignments {
                let cap = catalog
                    .iter()
                    .find(|c| c.capability_id == assignment.capability_id)
                    .unwrap();
                let identity = format!(
                    "{}#visit{}#{}",
                    assignment.node_id, update.run.activation, cap.capability_id
                );
                let op = LayeredOperation {
                    round: update.run.round,
                    activation: update.run.activation,
                    operation_id: operation_id(&update.run.run_id, *layer, &identity, 1),
                    execution_id: execution_id(&update.run.run_id, *layer, &identity, 1, cap.kind),
                    run_id: update.run.run_id.clone(),
                    layer: *layer,
                    node_id: assignment.node_id.clone(),
                    attempt: 1,
                    capability_id: cap.capability_id.clone(),
                    execution_kind: cap.kind,
                    status: LayeredOperationStatus::Creating,
                    source_sequence: None,
                    cancel_requested: false,
                };
                let mut e = event(
                    &update.run,
                    "node_dispatched",
                    Some(assignment.reason.clone()),
                );
                e.node_id = Some(op.node_id.clone());
                e.capability_id = Some(op.capability_id.clone());
                e.execution_id = Some(op.execution_id.clone());
                e.execution_kind = Some(op.execution_kind);
                update.events.push(e);
                update.operations.push(op);
            }
        }
        LayeredDecision::Complete {
            reason,
            evidence_execution_ids,
            summary,
            assessments,
        } => {
            validate::reason(reason)?;
            validate::evidence(evidence_execution_ids, &snapshot.operations)?;
            ensure!(
                snapshot.run.layer as usize == groups.len()
                    && snapshot.run.valid_layers + 1 == snapshot.run.layer
                    && terminal::current_successful(snapshot),
                "all effective layers must pass before completion"
            );
            ensure!(summary.chars().count() <= 16384, "summary too large");
            assess(&mut update, snapshot, request, assessments, true)?;
            update.run.valid_layers = snapshot.run.layer;
            update.run.phase = LayeredPhase::Completed;
            update.run.summary = Some(summary.clone());
            let mut e = event(&update.run, "run_completed", Some(reason.clone()));
            e.decision_summary = Some("complete".into());
            e.evidence_execution_ids = evidence_execution_ids.clone();
            update.events.push(e);
        }
        LayeredDecision::Block { reason } => {
            validate::reason(reason)?;
            return Ok(super::block(snapshot, reason.clone(), now));
        }
        LayeredDecision::Fail { reason, error_type } => {
            validate::reason(reason)?;
            validate::reason(error_type)?;
            update.run.phase = LayeredPhase::Failed;
            update.run.error = Some(format!("{error_type}: {reason}"));
            update
                .events
                .push(event(&update.run, "run_failed", update.run.error.clone()));
        }
    }
    Ok(update)
}

fn assess(
    update: &mut LayeredChange,
    snapshot: &LayeredSnapshot,
    request: &LayeredRequest,
    assessments: &std::collections::BTreeMap<String, MilestoneAssessment>,
    advancing: bool,
) -> Result<()> {
    let current: Vec<_> = request
        .plan
        .nodes
        .iter()
        .filter(|n| n.layer == snapshot.run.layer)
        .collect();
    ensure!(
        assessments.len() == current.len()
            && current.iter().all(|n| assessments.contains_key(&n.node_id)),
        "assess every current milestone exactly once"
    );
    for (id, verdict) in assessments {
        validate::reason(&verdict.reason)?;
        ensure!(
            !advancing || verdict.met,
            "milestone {id} did not meet its criteria"
        );
        if verdict.met {
            ensure!(
                snapshot
                    .operations
                    .iter()
                    .filter(|op| op.activation == snapshot.run.activation && &op.node_id == id)
                    .all(|op| op.status.successful()),
                "failed execution cannot count as a met milestone"
            );
        }
    }
    if !current.is_empty() {
        let mut evaluated = event(&snapshot.run, "milestones_assessed", None);
        evaluated.at_ms = update.run.updated_at;
        evaluated.assessments = assessments.clone();
        update.events.push(evaluated);
    }
    Ok(())
}
