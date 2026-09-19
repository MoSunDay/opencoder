use super::{change, event, validation};
use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::*, fleet::ExecutionKind};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub fn execution_id(run: &str, round: u32, capability: &str, kind: ExecutionKind) -> String {
    format!(
        "{}-{:x}",
        kind.prefix(),
        Sha256::digest(format!("{run}\0{round}\0{capability}").as_bytes())
    )[..kind.prefix().len() + 41]
        .into()
}
pub fn decide(
    snapshot: &BrainSchedulerSnapshot,
    request: &BrainSchedulerRequest,
    catalog: &[BrainCapabilityDescriptor],
    decision: &BrainSchedulerDecision,
    now: i64,
) -> Result<BrainSchedulerChange> {
    ensure!(
        snapshot.run.phase == BrainSchedulerPhase::Deciding,
        "run is not deciding"
    );
    ensure!(
        snapshot.operations.iter().all(|o| o.status.successful()),
        "decision requires a successful round barrier"
    );
    let mut update = change(snapshot, now);
    match decision {
        BrainSchedulerDecision::Dispatch {
            capabilities,
            reason,
            evidence_execution_ids,
        } => {
            validation::reason(reason)?;
            validation::evidence(evidence_execution_ids, &snapshot.operations)?;
            ensure!(
                snapshot.run.round < request.max_rounds,
                "maximum scheduler rounds exceeded"
            );
            ensure!(
                !capabilities.is_empty() && capabilities.len() <= 32,
                "dispatch requires 1..32 capabilities"
            );
            let mut ids = BTreeSet::new();
            update.run.round += 1;
            update.run.phase = BrainSchedulerPhase::Waiting;
            let mut started = event(&update.run, "round_started", Some(reason.clone()));
            started.decision_summary = Some("dispatch".into());
            started.evidence_execution_ids = evidence_execution_ids.clone();
            update.events.push(started);
            for item in capabilities {
                ensure!(ids.insert(&item.capability_id), "duplicate capability");
                let cap = catalog
                    .iter()
                    .find(|c| c.capability_id == item.capability_id)
                    .context("illegal capability")?;
                ensure!(
                    super::prefilter(std::slice::from_ref(cap), request).len() == 1,
                    "capability is unavailable or lacks descriptions"
                );
                validation::bindings(item, cap, request, &snapshot.operations)?;
                let id = execution_id(
                    &snapshot.run.run_id,
                    update.run.round,
                    &cap.capability_id,
                    cap.kind,
                );
                let op = BrainOperation {
                    operation_id: id.clone(),
                    run_id: snapshot.run.run_id.clone(),
                    round: update.run.round,
                    capability_id: cap.capability_id.clone(),
                    execution_kind: cap.kind,
                    execution_id: id,
                    status: BrainOperationStatus::Creating,
                    source_sequence: None,
                    cancel_requested: false,
                };
                let mut created = event(&update.run, "operation_created", None);
                created.capability_id = Some(op.capability_id.clone());
                created.execution_kind = Some(op.execution_kind);
                created.execution_id = Some(op.execution_id.clone());
                update.events.push(created);
                update.operations.push(op);
            }
        }
        BrainSchedulerDecision::Complete {
            reason,
            evidence_execution_ids,
        } => {
            validation::reason(reason)?;
            validation::evidence(evidence_execution_ids, &snapshot.operations)?;
            ensure!(
                snapshot.run.round > 0
                    && snapshot.run.round <= request.max_rounds
                    && !evidence_execution_ids.is_empty(),
                "completion requires successful execution evidence within round limit"
            );
            update.run.phase = BrainSchedulerPhase::Completed;
            let mut e = event(&update.run, "run_completed", Some(reason.clone()));
            e.decision_summary = Some("complete".into());
            e.evidence_execution_ids = evidence_execution_ids.clone();
            update.events.push(e);
        }
        BrainSchedulerDecision::Fail { reason, error_type } => {
            validation::reason(reason)?;
            validation::reason(error_type)?;
            update.run.phase = BrainSchedulerPhase::Failed;
            update.run.error = Some(error_type.clone());
            update
                .events
                .push(event(&update.run, "run_failed", Some(reason.clone())));
        }
    }
    Ok(update)
}
