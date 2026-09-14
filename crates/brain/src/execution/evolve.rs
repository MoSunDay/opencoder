use super::{fingerprint, resolve, Resolution};
use anyhow::{ensure, Result};
use opencoder_core::brain::*;
use serde_json::Value;
use std::collections::BTreeMap;

pub fn advance(run: &mut BrainRun, now: i64) -> Result<()> {
    run.updated_at = now;
    if run.phase.terminal()
        || run.phase == RunPhase::Planning
        || (run.phase == RunPhase::Paused && run.request.plan.is_none())
    {
        return Ok(());
    }
    if run.phase == RunPhase::Cancelling {
        if !run.instances.values().any(|i| i.status.active()) {
            run.phase = RunPhase::Cancelled;
        }
        return Ok(());
    }
    let plan = run
        .request
        .plan
        .as_ref()
        .expect("active run has plan")
        .plan
        .clone();
    for (name, input) in &plan.inputs {
        if let Some(value) = run.request.inputs.get(name) {
            crate::ontology::accepts(&input.schema, value)?;
        } else if input.required {
            run.input_requests
                .entry(name.clone())
                .or_insert_with(|| InputRequest {
                    name: name.clone(),
                    description: input.description.clone(),
                    schema: input.schema.clone(),
                    answered: false,
                });
        }
    }
    if let Some(flow) = &plan.flow {
        return super::flow::advance(run, &plan, flow, now);
    }
    loop {
        let before = serde_json::to_value((&run.instances, &run.expansions))?;
        for step in &plan.steps {
            if !run.expansions.contains_key(&step.id) {
                expand(run, step, now)?;
            }
            for id in run.expansions.get(&step.id).cloned().unwrap_or_default() {
                if run.instances[&id].status != StepStatus::Waiting {
                    continue;
                }
                let item = run.instances[&id].item.clone();
                let (status, inputs, reason) = readiness(run, step, &item);
                let instance = run.instances.get_mut(&id).unwrap();
                instance.status = status;
                instance.inputs = inputs;
                instance.reason = reason;
                if status.terminal() {
                    instance.finished_at = Some(now);
                }
            }
        }
        if serde_json::to_value((&run.instances, &run.expansions))? == before {
            break;
        }
    }
    if run.phase == RunPhase::Paused {
        return Ok(());
    }
    let pending_inputs = run.input_requests.values().any(|i| !i.answered);
    if run.expansions.len() == plan.steps.len()
        && run.instances.values().all(|i| i.status.terminal())
        && !pending_inputs
    {
        for (name, delivery) in &plan.deliverables {
            match resolve(run, &delivery.source, None) {
                Resolution::Value(value) => {
                    if let Err(e) = crate::ontology::accepts(&delivery.schema, &value) {
                        return fail(run, format!("deliverable {name}: {e}"));
                    }
                    if delivery.expected.as_ref().is_some_and(|v| *v != value) {
                        return fail(run, format!("deliverable {name} failed verification"));
                    }
                    run.deliverables.insert(name.clone(), value);
                }
                Resolution::Unavailable(reason) | Resolution::Waiting(reason) => {
                    return fail(run, format!("deliverable {name}: {reason}"))
                }
            }
        }
        if run
            .instances
            .values()
            .any(|i| matches!(i.status, StepStatus::Failed | StepStatus::Cancelled))
        {
            return fail(run, "required steps failed".into());
        }
        run.phase = RunPhase::Completed;
    } else if run
        .instances
        .values()
        .any(|i| i.status.active() || i.status == StepStatus::Ready)
    {
        run.phase = RunPhase::Running;
    } else if pending_inputs {
        run.phase = RunPhase::WaitingInput;
    } else {
        return fail(
            run,
            "plan cannot progress and has no input, child execution, or resource wake source"
                .into(),
        );
    }
    Ok(())
}

fn fail(run: &mut BrainRun, message: String) -> Result<()> {
    run.phase = RunPhase::Failed;
    run.error = Some(message);
    Ok(())
}

fn expand(run: &mut BrainRun, step: &StepTemplate, now: i64) -> Result<()> {
    let mut items = vec![(None, Value::Null)];
    if let Some(expansion) = &step.expansion {
        items = match resolve(run, &expansion.items, None) {
            Resolution::Waiting(_) => return Ok(()),
            Resolution::Unavailable(reason) => return seal_error(run, step, &reason, now),
            Resolution::Value(value) => {
                let Some(array) = value.as_array() else {
                    return seal_error(run, step, "foreach input is not an array", now);
                };
                if array.is_empty() && !expansion.allow_empty {
                    return seal_error(run, step, "empty expansion forbidden", now);
                }
                let mut keys = std::collections::BTreeSet::new();
                let mut entries = vec![];
                for value in array {
                    let key = value.pointer(&expansion.key);
                    if !key.is_some_and(|v| v.is_string() || v.is_i64() || v.is_u64()) {
                        return seal_error(run, step, "foreach item has no scalar identity", now);
                    }
                    let key = serde_json::to_string(key.unwrap())?;
                    if !keys.insert(key.clone()) {
                        return seal_error(run, step, "duplicate foreach item identity", now);
                    }
                    entries.push((Some(key), value.clone()));
                }
                entries
            }
        };
    }
    ensure!(
        run.instances.len() + items.len() <= 10_000,
        "plan exceeds 10000 expanded instances"
    );
    let mut ids = Vec::new();
    for (key, item) in items {
        let id = key
            .as_ref()
            .map(|k| format!("{}~{}", step.id, &fingerprint(k)[..16]))
            .unwrap_or_else(|| step.id.clone());
        ids.push(id.clone());
        run.instances
            .insert(id.clone(), instance(id, step, key, item));
    }
    run.expansions.insert(step.id.clone(), ids);
    Ok(())
}

fn seal_error(run: &mut BrainRun, step: &StepTemplate, reason: &str, now: i64) -> Result<()> {
    let mut failed = instance(step.id.clone(), step, None, Value::Null);
    failed.status = StepStatus::Failed;
    failed.reason = reason.into();
    failed.finished_at = Some(now);
    run.instances.insert(step.id.clone(), failed);
    run.expansions
        .insert(step.id.clone(), vec![step.id.clone()]);
    Ok(())
}

pub(super) fn instance(
    id: String,
    step: &StepTemplate,
    key: Option<String>,
    item: Value,
) -> StepInstance {
    StepInstance {
        id,
        step_id: step.id.clone(),
        item_key: key,
        item,
        status: StepStatus::Waiting,
        attempt: 0,
        execution: None,
        node_id: None,
        inputs: BTreeMap::new(),
        output: None,
        reason: String::new(),
        started_at: None,
        finished_at: None,
    }
}

fn readiness(
    run: &BrainRun,
    step: &StepTemplate,
    item: &Value,
) -> (StepStatus, BTreeMap<String, Value>, String) {
    let mut inputs = BTreeMap::new();
    for dep in &step.depends_on {
        let Some(ids) = run.expansions.get(dep) else {
            return (StepStatus::Waiting, inputs, format!("waiting for {dep}"));
        };
        for id in ids {
            let instance = &run.instances[id];
            if !instance.status.terminal() {
                return (StepStatus::Waiting, inputs, format!("waiting for {id}"));
            }
            if matches!(instance.status, StepStatus::Failed | StepStatus::Cancelled) {
                return (
                    StepStatus::Failed,
                    inputs,
                    format!("dependency {id} failed"),
                );
            }
        }
    }
    if let Some(condition) = &step.when {
        match resolve(run, &condition.value, Some(item)) {
            Resolution::Value(value) if value != condition.equals => {
                return (StepStatus::Skipped, inputs, "condition is false".into())
            }
            Resolution::Waiting(reason) => return (StepStatus::Waiting, inputs, reason),
            Resolution::Unavailable(reason) => return (StepStatus::Failed, inputs, reason),
            _ => {}
        }
    }
    for (name, input) in &step.inputs {
        match resolve(run, &input.binding, Some(item)) {
            Resolution::Value(value) => {
                if let Err(e) = crate::ontology::accepts(&input.schema, &value) {
                    return (StepStatus::Failed, inputs, format!("{name}: {e}"));
                }
                inputs.insert(name.clone(), value);
            }
            Resolution::Waiting(reason) if input.required || !reason.starts_with("input:") => {
                return (StepStatus::Waiting, inputs, reason)
            }
            Resolution::Unavailable(reason) if input.required => {
                return (StepStatus::Failed, inputs, reason)
            }
            _ => {}
        }
    }
    (StepStatus::Ready, inputs, String::new())
}
