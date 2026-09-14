use super::{evolve::instance, resolve, Resolution};
use anyhow::Result;
use opencoder_core::brain::*;
use serde_json::Value;
use std::collections::BTreeMap;

fn fail(run: &mut BrainRun, message: impl Into<String>) {
    run.phase = RunPhase::Failed;
    run.error = Some(message.into());
}
fn enter(run: &mut BrainRun, plan: &OntologyPlan, flow: &ActionFlow, target: &str) {
    let visits = run
        .instances
        .values()
        .filter(|i| i.step_id == target)
        .count()
        + 1;
    if visits > flow.max_visits_per_action as usize || run.instances.len() >= 10_000 {
        fail(run, format!("action {target} exceeded visit limit"));
        return;
    }
    let Some(step) = plan.steps.iter().find(|s| s.id == target) else {
        fail(run, "unknown flow target");
        return;
    };
    let id = format!("{target}~visit-{visits:04}");
    run.instances
        .insert(id.clone(), instance(id.clone(), step, None, Value::Null));
    run.flow_current = Some(id);
    run.revision += 1;
}
pub fn advance(run: &mut BrainRun, plan: &OntologyPlan, flow: &ActionFlow, now: i64) -> Result<()> {
    if run.phase == RunPhase::Cancelling {
        if !run.instances.values().any(|i| i.status.active()) {
            run.phase = RunPhase::Cancelled;
        }
        run.updated_at = now;
        return Ok(());
    }
    if run.phase == RunPhase::Paused || run.phase.terminal() {
        return Ok(());
    }
    if run.flow_current.is_none() {
        enter(run, plan, flow, &flow.entry);
    }
    if run.phase.terminal() {
        return Ok(());
    }
    let id = run.flow_current.clone().unwrap();
    let step_id = run.instances[&id].step_id.clone();
    let step = plan.steps.iter().find(|s| s.id == step_id).unwrap();
    match run.instances[&id].status {
        StepStatus::Waiting => {
            let mut inputs = BTreeMap::new();
            for (name, port) in &step.inputs {
                if !port.required
                    && matches!(&port.binding, Binding::Output { step, .. } if !run.expansions.contains_key(step))
                {
                    continue;
                }
                match resolve(run, &port.binding, None) {
                    Resolution::Value(v) => {
                        if let Err(e) = crate::ontology::accepts(&port.schema, &v) {
                            fail(run, format!("input {name}: {e}"));
                            return Ok(());
                        }
                        inputs.insert(name.clone(), v);
                    }
                    Resolution::Waiting(reason) if reason.starts_with("input:") => {
                        if port.required {
                            run.phase = RunPhase::WaitingInput;
                            return Ok(());
                        }
                    }
                    Resolution::Waiting(reason) | Resolution::Unavailable(reason) => {
                        fail(run, format!("input {name}: {reason}"));
                        return Ok(());
                    }
                }
            }
            if run.input_requests.values().any(|r| !r.answered) {
                run.phase = RunPhase::WaitingInput;
                return Ok(());
            }
            let i = run.instances.get_mut(&id).unwrap();
            i.inputs = inputs;
            i.status = StepStatus::Ready;
            run.expansions.insert(step.id.clone(), vec![id.clone()]);
            run.phase = RunPhase::Running;
        }
        StepStatus::Succeeded => {
            let edges: Vec<_> = flow
                .transitions
                .iter()
                .filter(|e| e.from == step.id)
                .collect();
            let mut matched = vec![];
            for edge in &edges {
                if let Some(c) = &edge.when {
                    match resolve(run, &c.value, None) {
                        Resolution::Value(v) => {
                            if v == c.equals {
                                matched.push(*edge);
                            }
                        }
                        Resolution::Waiting(reason) | Resolution::Unavailable(reason) => {
                            fail(run, format!("transition {}: {reason}", edge.label));
                            return Ok(());
                        }
                    }
                }
            }
            if matched.len() > 1 {
                fail(
                    run,
                    format!("action {} matched multiple transitions", step.id),
                );
                return Ok(());
            }
            let edge = matched
                .first()
                .copied()
                .or_else(|| edges.iter().copied().find(|e| e.when.is_none()));
            let Some(edge) = edge else {
                fail(
                    run,
                    format!("action {} has no matching transition", step.id),
                );
                return Ok(());
            };
            if let Some(target) = &edge.to {
                enter(run, plan, flow, target);
                if !run.phase.terminal() {
                    advance(run, plan, flow, now)?;
                }
            } else {
                for (name, d) in &plan.deliverables {
                    match resolve(run, &d.source, None) {
                        Resolution::Value(v) => {
                            if let Err(e) = crate::ontology::accepts(&d.schema, &v) {
                                fail(run, format!("deliverable {name}: {e}"));
                                return Ok(());
                            }
                            if d.expected.as_ref().is_some_and(|x| x != &v) {
                                fail(run, format!("deliverable {name} failed verification"));
                                return Ok(());
                            }
                            run.deliverables.insert(name.clone(), v);
                        }
                        Resolution::Waiting(r) | Resolution::Unavailable(r) => {
                            fail(run, format!("deliverable {name}: {r}"));
                            return Ok(());
                        }
                    }
                }
                run.phase = RunPhase::Completed;
            }
        }
        StepStatus::Failed | StepStatus::Cancelled | StepStatus::Skipped => {
            fail(run, format!("action {} did not succeed", step.id))
        }
        _ => run.phase = RunPhase::Running,
    }
    run.updated_at = now;
    Ok(())
}
