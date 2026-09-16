use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::*, fleet::*};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

pub fn fingerprint(value: &impl Serialize) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("serializable brain data"))
    )
}

pub fn context(run: &mut BrainRun) -> ActivationContext {
    run.activation += 1;
    ActivationContext {
        schema_version: run.request.schema_version,
        routes: crate::graph::pending(run),
        run_id: run.id.clone(),
        activation: run.activation,
        control_epoch: run.control_epoch,
        revision: run.revision,
        plan_ref: run.request.plan.as_ref().map(|p| PlanRef {
            id: p.id.clone(),
            version: p.version,
        }),
        plan: run.request.plan.as_ref().map(|p| p.plan.clone()),
        objective: run.request.objective.clone(),
        phase: run.phase,
        inputs: run.request.inputs.clone(),
        instances: run.instances.values().cloned().collect(),
        ready: run
            .instances
            .values()
            .filter(|i| i.status == StepStatus::Ready)
            .map(|i| i.id.clone())
            .collect(),
        references: run.request.references.clone(),
        capabilities: run.request.capabilities.clone(),
    }
}

pub fn decide(run: &BrainRun, decision: &ActivationDecision) -> Result<()> {
    ensure!(
        decision.run_id == run.id
            && decision.activation == run.activation
            && decision.control_epoch == run.control_epoch,
        "stale activation or control epoch"
    );
    ensure!(
        !matches!(run.phase, RunPhase::Paused | RunPhase::Cancelling) && !run.phase.terminal(),
        "dispatch is suspended"
    );
    ensure!(
        run.request.plan.is_none() || decision.plan.is_none(),
        "runtime replanning is not enabled"
    );
    let mut receipts = std::collections::BTreeSet::new();
    for route in &decision.routes {
        ensure!(receipts.insert(&route.receipt), "duplicate routing receipt");
        ensure!(
            run.graph
                .routes
                .get(&route.receipt)
                .is_some_and(|r| r.decision.is_none()),
            "unknown or already decided route"
        );
    }
    let mut ids = std::collections::BTreeSet::new();
    for id in &decision.dispatch {
        ensure!(ids.insert(id), "duplicate dispatch id");
        ensure!(
            run.instances
                .get(id)
                .is_some_and(|i| i.status == StepStatus::Ready),
            "step {id} is no longer ready"
        );
    }
    Ok(())
}

pub fn prepare(run: &mut BrainRun, instance_id: &str, owner: &str, now: i64) -> Result<String> {
    ensure!(run.phase == RunPhase::Running, "dispatch is suspended");
    let instance = run
        .instances
        .get_mut(instance_id)
        .context("unknown instance")?;
    ensure!(instance.status == StepStatus::Ready, "instance not ready");
    let step = run
        .request
        .plan
        .as_ref()
        .context("plan is missing")?
        .plan
        .instances
        .iter()
        .find(|s| s.id == instance.step_id)
        .unwrap();
    instance.attempt += 1;
    let id = format!(
        "{}-{}",
        step.action.kind.prefix(),
        &fingerprint(&(&run.id, instance_id, instance.attempt))[..40]
    );
    let link = BrainLink {
        run_id: run.id.clone(),
        node_id: owner.into(),
        instance_id: instance_id.into(),
        attempt: instance.attempt,
    };
    let outputs: std::collections::BTreeMap<_, _> = step
        .outputs
        .iter()
        .map(|id| (id, &run.request.plan.as_ref().unwrap().plan.outputs[id]))
        .collect();
    let prompt = format!("{}\n\nInputs (immutable JSON):\n{}\n\nNamed output descriptions: {}\nReturn one JSON object keyed by output ID. Each value: {{\"content\":actual content,\"completion\":{{\"passed\":true/false/null,\"evidence\":[\"reason\"]}},\"verification\":{{\"passed\":true/false/null,\"evidence\":[\"reason\"]}}}}. Assessments without evidence are unknown. Do not infer verification from process completion.", step.action.prompt, serde_json::to_string(&instance.inputs)?, serde_json::to_string(&outputs)?);
    let mut action = step.action.clone();
    action.output_mode = OutputMode::Json;
    let mut input = json!({"prompt":prompt,"parameters":instance.inputs,"_brain":{"schema_version":2,"capability_id":step.capability_id,"parent":link,"action":action,"output_schema":crate::graph::output_schema(),"outputs":outputs,"resources":step.resources}});
    if let Some(definition) = &step.action.definition {
        input[if step.action.kind == ExecutionKind::Todos {
            "spec"
        } else {
            "definition"
        }] = definition.clone();
    }
    let request = CreateExecution {
        id: id.clone(),
        kind: step.action.kind,
        target: Some(step.action.target.clone()),
        input,
        node_id: step.action.node_id.clone(),
    };
    let receipt = ActionReceipt {
        id: id.clone(),
        instance_id: instance_id.into(),
        attempt: instance.attempt,
        kind: ActionKind::Execute,
        state: ReceiptState::Prepared,
        input_fingerprint: fingerprint(&instance.inputs),
        request: serde_json::to_value(&request)?,
        result: None,
        error: None,
    };
    instance.execution = Some(ExecutionRef {
        id: id.clone(),
        kind: step.action.kind,
    });
    instance.status = StepStatus::Queued;
    instance.started_at = Some(now);
    run.actions.insert(id.clone(), receipt);
    run.revision += 1;
    Ok(id)
}

pub fn accept_action(run: &mut BrainRun, id: &str, reply: &RpcReply, now: i64) -> Result<()> {
    let receipt = run.actions.get_mut(id).context("unknown action receipt")?;
    if receipt.state == ReceiptState::Settled {
        return Ok(());
    }
    // Transport failures are uncertain outcomes. Keep the original action id
    // until the owner provides an authoritative acceptance/rejection receipt.
    if reply.status >= 500 || reply.status == 408 || reply.status == 429 || reply.status == 423 {
        let error = reply.body.to_string();
        if receipt.error.as_ref() != Some(&error) {
            receipt.error = Some(error.clone());
            if let Some(instance) = run.instances.get_mut(&receipt.instance_id) {
                instance.reason = error;
            }
            run.revision += 1;
        }
        return Ok(());
    }
    let instance = run
        .instances
        .get_mut(&receipt.instance_id)
        .context("missing instance")?;
    receipt.result = Some(reply.body.clone());
    if (200..300).contains(&reply.status) {
        receipt.state = ReceiptState::Accepted;
        receipt.error = None;
        instance.reason.clear();
        instance.node_id = reply.body["node_id"]
            .as_str()
            .map(str::to_owned)
            .or(instance.node_id.clone());
    } else {
        receipt.state = ReceiptState::Settled;
        receipt.error = Some(reply.body.to_string());
        instance.status = StepStatus::Failed;
        instance.reason = reply.body.to_string();
        instance.finished_at = Some(now);
    }
    run.revision += 1;
    super::advance(run, now)
}

pub fn prepare_cancel(run: &mut BrainRun, instance_id: &str) -> Result<String> {
    ensure!(run.phase == RunPhase::Cancelling, "run is not cancelling");
    let instance = run.instances.get(instance_id).context("unknown instance")?;
    let execution = instance
        .execution
        .as_ref()
        .context("instance has no execution")?;
    let id = format!("cancel:{}", execution.id);
    run.actions
        .entry(id.clone())
        .or_insert_with(|| ActionReceipt {
            id: id.clone(),
            instance_id: instance_id.into(),
            attempt: instance.attempt,
            kind: ActionKind::Cancel,
            state: ReceiptState::Prepared,
            input_fingerprint: fingerprint(&instance.inputs),
            request: json!(execution),
            result: None,
            error: None,
        });
    Ok(id)
}
