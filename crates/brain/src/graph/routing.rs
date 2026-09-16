use super::advance::{block, enter};
use anyhow::{ensure, Context, Result};
use opencoder_core::brain::*;
use std::collections::{BTreeMap, BTreeSet};

pub fn pending(run: &BrainRun) -> Vec<RouteContext> {
    run.graph
        .routes
        .values()
        .filter(|r| r.decision.is_none())
        .map(|r| r.context.clone())
        .collect()
}

fn outgoing<'a>(plan: &'a OntologyPlan, step: &str) -> &'a GraphRoute {
    let outputs = &plan
        .instances
        .iter()
        .find(|i| i.id == step)
        .unwrap()
        .outputs;
    plan.routes
        .iter()
        .find(|r| r.outputs.iter().any(|o| outputs.contains(o)))
        .unwrap()
}

// A live branch can still contribute to this join until it selects a bypass or
// an exit. Stop traversal at the join itself, so a future loop is not an ancestor.
fn reaches(plan: &OntologyPlan, step: &str, route: &str, seen: &mut BTreeSet<String>) -> bool {
    if !seen.insert(step.into()) {
        return false;
    }
    let next = outgoing(plan, step);
    next.id == route
        || next
            .targets
            .iter()
            .any(|t| reaches(plan, &t.instance, route, seen))
}

pub(super) fn prepare(run: &mut BrainRun) -> Result<()> {
    let plan = &run.request.plan.as_ref().context("plan missing")?.plan;
    for route in &plan.routes {
        let producers: BTreeSet<_> = plan
            .instances
            .iter()
            .filter(|i| i.outputs.iter().any(|o| route.outputs.contains(o)))
            .map(|i| &i.id)
            .collect();
        let cohorts: Vec<Vec<String>> = if producers.len() == 1 {
            run.graph
                .tokens
                .keys()
                .filter(|id| {
                    producers.contains(&run.instances[*id].step_id)
                        && run.instances[*id].status == StepStatus::Succeeded
                })
                .map(|id| vec![id.clone()])
                .collect()
        } else {
            let relevant = run
                .graph
                .tokens
                .keys()
                .filter(|id| {
                    reaches(
                        plan,
                        &run.instances[*id].step_id,
                        &route.id,
                        &mut BTreeSet::new(),
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            super::causal::cohorts(run, &relevant, &producers)
        };
        for participants in cohorts {
            if participants.is_empty() {
                continue;
            }
            if !participants.iter().all(|id| {
                let instance = &run.instances[id];
                instance.status == StepStatus::Succeeded
                    && outgoing(plan, &instance.step_id).id == route.id
            }) {
                continue;
            }
            let receipt = format!(
                "route-{}",
                &crate::execution::fingerprint(&(&route.id, &participants))[..32]
            );
            if run.graph.routes.contains_key(&receipt) {
                continue;
            }
            let mut outputs = vec![];
            let mut names = BTreeSet::new();
            for id in &participants {
                let definition = plan
                    .instances
                    .iter()
                    .find(|i| i.id == run.instances[id].step_id)
                    .unwrap();
                for output in route
                    .outputs
                    .iter()
                    .filter(|o| definition.outputs.contains(o))
                {
                    ensure!(names.insert(output.clone()), "ambiguous concurrent producers of {output}; join their outputs before re-entering the same instance");
                    outputs.push(
                        run.graph
                            .outputs
                            .get(&format!("{id}/{output}"))
                            .with_context(|| {
                                format!("required connected output {output} missing from {id}")
                            })?
                            .clone(),
                    );
                }
            }
            let context = RouteContext {
                receipt: receipt.clone(),
                route: route.id.clone(),
                description: route.description.clone(),
                outputs,
                candidates: route
                    .targets
                    .iter()
                    .map(|target| RouteCandidate {
                        instance: target.instance.clone(),
                        inputs: plan
                            .instances
                            .iter()
                            .find(|i| i.id == target.instance)
                            .unwrap()
                            .inputs
                            .iter()
                            .map(|id| (id.clone(), plan.inputs[id].description.clone()))
                            .collect(),
                    })
                    .collect(),
                exits: route.exits.clone(),
            };
            run.graph.routes.insert(
                receipt,
                RouteReceipt {
                    context,
                    tokens: participants,
                    decision: None,
                },
            );
            run.revision += 1;
        }
    }
    Ok(())
}

pub fn apply(run: &mut BrainRun, decision: &RouteDecision, now: i64) -> Result<()> {
    let receipt = run
        .graph
        .routes
        .get(&decision.receipt)
        .context("unknown route receipt")?
        .clone();
    if let Some(previous) = &receipt.decision {
        ensure!(previous == decision, "route decision is immutable");
        return Ok(());
    }
    ensure!(run.phase == RunPhase::Running, "routing is suspended");
    // Validate and apply on a copy: invalid model output records a block and
    // never leaves a partially consumed join or partially dispatched split.
    let mut next = run.clone();
    next.graph
        .routes
        .get_mut(&decision.receipt)
        .unwrap()
        .decision = Some(decision.clone());
    match select(&mut next, &receipt, decision) {
        Ok(()) => {
            next.revision += 1;
            *run = next;
            super::advance(run, now)?;
        }
        Err(error) => {
            run.graph
                .routes
                .get_mut(&decision.receipt)
                .unwrap()
                .decision = Some(decision.clone());
            block(run, format!("route {}: {error:#}", receipt.context.route));
        }
    }
    Ok(())
}

fn select(run: &mut BrainRun, receipt: &RouteReceipt, decision: &RouteDecision) -> Result<()> {
    ensure!(
        !decision.reason.trim().is_empty(),
        "route decision needs evidence/reason"
    );
    if let Some(reason) = &decision.blocked {
        anyhow::bail!("model blocked: {reason}");
    }
    ensure!(
        decision.exit.is_some() == decision.selected.is_empty(),
        "choose adjacent targets or one explicit exit"
    );
    let plan = run.request.plan.as_ref().unwrap().plan.clone();
    let route = plan
        .routes
        .iter()
        .find(|r| r.id == receipt.context.route)
        .unwrap();
    ensure!(
        receipt
            .tokens
            .iter()
            .all(|id| run.graph.tokens.contains_key(id)),
        "causal input already consumed"
    );
    let outputs: BTreeMap<_, _> = receipt
        .context
        .outputs
        .iter()
        .map(|o| (o.output.as_str(), o))
        .collect();
    if let Some(exit_id) = &decision.exit {
        let exit = route
            .exits
            .iter()
            .find(|e| e.id == *exit_id)
            .context("undeclared exit")?;
        for name in &exit.deliverables {
            let output = outputs
                .get(name.as_str())
                .with_context(|| format!("missing delivery {name}"))?;
            if exit.require_completed {
                proven(&output.value.completion, "completion")?;
            }
            if exit.require_verified {
                proven(&output.value.verification, "verification")?;
            }
            run.deliverables.insert(
                format!("{}/{name}", decision.receipt),
                output.value.content.clone(),
            );
        }
        run.graph
            .exits
            .push(format!("{}/{exit_id}", decision.receipt));
    } else {
        let mut selected = BTreeSet::new();
        for target_id in &decision.selected {
            ensure!(selected.insert(target_id), "duplicate selected target");
            let target = route
                .targets
                .iter()
                .find(|t| t.instance == *target_id)
                .context("non-adjacent target selected")?;
            let mut refs = BTreeMap::new();
            for (input, output) in &target.bindings {
                if let Some(record) = outputs.get(output.as_str()) {
                    refs.insert(input.clone(), record.id.clone());
                } else {
                    ensure!(
                        !plan.inputs[input].required,
                        "selected target requires missing output {output}"
                    );
                }
            }
            let mut scope = super::causal::child_scope(run, &receipt.tokens);
            if decision.selected.len() > 1 {
                scope.push(GraphScope {
                    fork: decision.receipt.clone(),
                    branch: target_id.clone(),
                });
            }
            enter(run, target_id, receipt.tokens.clone(), refs, scope)?;
        }
    }
    for id in &receipt.tokens {
        run.graph.tokens.remove(id);
    }
    Ok(())
}
fn proven(assessment: &Assessment, name: &str) -> Result<()> {
    ensure!(
        assessment.passed == Some(true) && assessment.evidence.iter().any(|e| !e.trim().is_empty()),
        "{name} is false or unknown; explicit output evidence required"
    );
    Ok(())
}
