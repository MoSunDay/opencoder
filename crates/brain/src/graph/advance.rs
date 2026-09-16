use anyhow::{Context, Result};
use opencoder_core::brain::*;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn block(run: &mut BrainRun, reason: impl Into<String>) {
    run.error = Some(reason.into());
    if run.phase != RunPhase::Paused {
        run.phase = RunPhase::Blocked;
    }
    run.revision += 1;
}

pub(super) fn enter(
    run: &mut BrainRun,
    target: &str,
    parents: Vec<String>,
    refs: BTreeMap<String, String>,
    scope: Vec<GraphScope>,
) -> Result<()> {
    let definition = run
        .request
        .plan
        .as_ref()
        .context("plan missing")?
        .plan
        .instances
        .iter()
        .find(|i| i.id == target)
        .context("unknown target")?
        .clone();
    let round = run.expansions.get(target).map_or(0, Vec::len) + 1;
    anyhow::ensure!(
        round <= definition.max_visits as usize && run.instances.len() < 10_000,
        "instance {target} exceeded visit limit"
    );
    let id = format!("{target}~visit-{round:04}");
    run.instances.insert(
        id.clone(),
        StepInstance {
            id: id.clone(),
            step_id: target.into(),
            item_key: Some(round.to_string()),
            item: Value::Null,
            status: StepStatus::Waiting,
            attempt: 0,
            execution: None,
            node_id: None,
            inputs: BTreeMap::new(),
            output: None,
            reason: String::new(),
            started_at: None,
            finished_at: None,
        },
    );
    run.expansions
        .entry(target.into())
        .or_default()
        .push(id.clone());
    let token = GraphToken {
        visit: id.clone(),
        parents,
        inputs: refs,
        scope,
    };
    run.graph.visits.insert(id.clone(), token.clone());
    run.graph.tokens.insert(id, token);
    run.revision += 1;
    Ok(())
}

pub fn advance(run: &mut BrainRun, now: i64) -> Result<()> {
    run.updated_at = now;
    if run.phase.terminal()
        || matches!(
            run.phase,
            RunPhase::Planning | RunPhase::Paused | RunPhase::Blocked
        )
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
        .context("active run has no plan")?
        .plan
        .clone();
    anyhow::ensure!(plan.schema_version == 2, "{}", super::MIGRATION);
    for (name, port) in &plan.inputs {
        if port.source != InputSource::External {
            continue;
        }
        if let Some(value) = run.request.inputs.get(name) {
            crate::ontology::accepts(&port.schema, value)?;
            if let Some(doc) = value.as_object().filter(|d| d.contains_key("markdown")) {
                anyhow::ensure!(
                    doc.get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty())
                        && doc["markdown"].is_string(),
                    "document needs name and Markdown body"
                );
            }
        }
    }
    if !run.graph.started {
        run.graph.started = true;
        for target in &plan.entry {
            let scope = if plan.entry.len() > 1 {
                vec![GraphScope {
                    fork: "entry".into(),
                    branch: target.clone(),
                }]
            } else {
                vec![]
            };
            enter(run, target, vec![], BTreeMap::new(), scope)?;
        }
    }
    for token in run.graph.tokens.values().cloned().collect::<Vec<_>>() {
        let visit = &token.visit;
        let instance = &run.instances[visit];
        if matches!(instance.status, StepStatus::Failed | StepStatus::Cancelled) {
            block(run, format!("instance {visit}: {}", instance.reason));
            return Ok(());
        }
        if instance.status == StepStatus::Succeeded {
            if let Err(error) = super::capture(run, visit) {
                block(run, format!("output contract for {visit}: {error:#}"));
                return Ok(());
            }
        }
        if run.instances[visit].status != StepStatus::Waiting {
            continue;
        }
        let definition = plan
            .instances
            .iter()
            .find(|i| i.id == run.instances[visit].step_id)
            .unwrap();
        let mut inputs = BTreeMap::new();
        let mut waiting = vec![];
        for name in &definition.inputs {
            let port = &plan.inputs[name];
            let value = if port.source == InputSource::External {
                run.request.inputs.get(name).cloned()
            } else {
                token
                    .inputs
                    .get(name)
                    .and_then(|id| run.graph.outputs.get(id))
                    .map(|o| o.value.content.clone())
            };
            match value {
                Some(value) => {
                    if let Err(error) = crate::ontology::accepts(&port.schema, &value) {
                        block(run, format!("input {name} for {visit}: {error}"));
                        return Ok(());
                    }
                    inputs.insert(name.clone(), value);
                }
                None if port.required => {
                    waiting.push(name.clone());
                    if port.source == InputSource::External {
                        run.input_requests
                            .entry(name.clone())
                            .or_insert(InputRequest {
                                name: name.clone(),
                                description: port.description.clone(),
                                schema: port.schema.clone(),
                                answered: false,
                            });
                    }
                }
                None => {}
            }
        }
        let instance = run.instances.get_mut(visit).unwrap();
        instance.inputs = inputs;
        instance.reason = if waiting.is_empty() {
            String::new()
        } else {
            format!("waiting for inputs: {}", waiting.join(", "))
        };
        if waiting.is_empty() {
            instance.status = StepStatus::Ready;
        }
    }
    if let Err(error) = super::routing::prepare(run) {
        block(run, format!("routing: {error:#}"));
        return Ok(());
    }
    if run.graph.tokens.is_empty() {
        if run.graph.exits.is_empty() {
            block(run, "no declared exit reached");
        } else {
            run.phase = RunPhase::Completed;
        }
    } else if run
        .instances
        .values()
        .any(|i| i.status.active() || i.status == StepStatus::Ready)
        || !super::pending(run).is_empty()
    {
        run.phase = RunPhase::Running;
    } else if run.input_requests.values().any(|r| !r.answered) {
        run.phase = RunPhase::WaitingInput;
    } else {
        block(
            run,
            "activated branches cannot converge; no route, input or execution can make progress",
        );
    }
    Ok(())
}
