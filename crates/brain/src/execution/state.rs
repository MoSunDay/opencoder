use super::advance;
use anyhow::{ensure, Context, Result};
use opencoder_core::brain::*;
use serde_json::Value;
use std::collections::BTreeMap;

pub fn initialize(id: &str, request: BrainRequest, now: i64) -> Result<BrainRun> {
    ensure!(request.schema_version == 2, "{}", crate::graph::MIGRATION);
    ensure!(
        !request.objective.trim().is_empty(),
        "objective is required"
    );
    match request.mode {
        PlanningMode::Fixed => {
            crate::ontology::validate(
                &request
                    .plan
                    .as_ref()
                    .context("fixed mode requires an explicit plan version")?
                    .plan,
            )?;
        }
        PlanningMode::Dynamic => ensure!(
            request.plan.is_none(),
            "dynamic mode cannot implicitly execute a fixed plan; use references"
        ),
    }
    for name in request.inputs.keys() {
        if let Some(version) = &request.plan {
            ensure!(
                version
                    .plan
                    .inputs
                    .get(name)
                    .is_some_and(|p| p.source == InputSource::External),
                "undeclared external input {name}"
            );
        }
    }
    for reference in &request.references {
        crate::ontology::validate(&reference.plan)?;
    }
    let mut run = BrainRun {
        graph: GraphState::default(),
        id: id.into(),
        phase: if request.mode == PlanningMode::Fixed {
            RunPhase::Running
        } else {
            RunPhase::Planning
        },
        revision: 1,
        control_epoch: 1,
        activation: 0,
        handled_revision: 0,
        request,
        candidate_plan: None,
        instances: BTreeMap::new(),
        expansions: BTreeMap::new(),
        flow_current: None,
        source_cursors: BTreeMap::new(),
        actions: BTreeMap::new(),
        input_requests: BTreeMap::new(),
        deliverables: BTreeMap::new(),
        error: None,
        created_at: now,
        updated_at: now,
    };
    advance(&mut run, now)?;
    Ok(run)
}

pub fn adopt(run: &mut BrainRun, version: PlanVersion, now: i64) -> Result<()> {
    ensure!(
        run.request.plan.is_none() && run.phase == RunPhase::Planning,
        "running plan versions are immutable"
    );
    crate::ontology::validate(&version.plan)?;
    for name in run.request.inputs.keys() {
        ensure!(
            version
                .plan
                .inputs
                .get(name)
                .is_some_and(|p| p.source == InputSource::External),
            "generated plan omitted supplied external input {name}"
        );
    }
    run.request.plan = Some(version);
    run.phase = RunPhase::Running;
    run.revision += 1;
    advance(run, now)
}

pub fn command(run: &mut BrainRun, action: &str, now: i64) -> Result<()> {
    ensure!(
        !run.phase.terminal(),
        "run is terminal; create a new run to execute again"
    );
    match action {
        "pause" => {
            ensure!(run.phase != RunPhase::Cancelling, "run is cancelling");
            run.phase = RunPhase::Paused;
        }
        "resume" => {
            ensure!(
                run.phase == RunPhase::Paused,
                "only a paused run can resume"
            );
            run.phase = if run.request.plan.is_some() {
                RunPhase::Running
            } else {
                RunPhase::Planning
            };
        }
        "cancel" => {
            run.phase = RunPhase::Cancelling;
            for instance in run.instances.values_mut() {
                if !instance.status.active() && !instance.status.terminal() {
                    instance.status = StepStatus::Cancelled;
                    instance.finished_at = Some(now);
                }
            }
        }
        _ => anyhow::bail!("unsupported brain command {action}"),
    }
    run.control_epoch += 1;
    run.revision += 1;
    advance(run, now)
}

pub fn supply_input(run: &mut BrainRun, name: &str, value: Value, now: i64) -> Result<()> {
    ensure!(
        !run.phase.terminal() && run.phase != RunPhase::Cancelling,
        "run cannot accept inputs"
    );
    let request = run
        .input_requests
        .get_mut(name)
        .context("input has not been requested")?;
    if request.answered {
        ensure!(
            run.request.inputs.get(name) == Some(&value),
            "input is immutable after being supplied"
        );
        return Ok(());
    }
    crate::ontology::accepts(&request.schema, &value)?;
    if let Some(doc) = value.as_object().filter(|d| d.contains_key("markdown")) {
        ensure!(
            doc.get("name")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
                && doc["markdown"].is_string(),
            "document needs name and Markdown body"
        );
    }
    request.answered = true;
    run.request.inputs.insert(name.into(), value);
    run.revision += 1;
    advance(run, now)
}
