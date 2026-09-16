use super::{container, persistence};
use crate::{journal::Record, Worker};
use anyhow::{Context, Result};
use opencoder_brain::execution;
use opencoder_core::{brain::*, fleet::*, Config};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

pub fn preflight() -> Result<()> {
    anyhow::ensure!(
        opencoder_dag_runtime::sandbox::runc::runc_available(),
        "Brain requires runc"
    );
    container::cli_path()?;
    Ok(())
}

pub async fn run(
    worker: &Worker,
    record: &Record,
    config: Config,
    cancel: CancellationToken,
) -> Result<(ExecutionStatus, Value)> {
    let result = run_inner(worker, record, config, cancel.clone()).await;
    if let Err(error) = &result {
        if !cancel.is_cancelled() {
            let gate = worker.lifecycle_gate(&record.assignment.index.id).await;
            let _guard = gate.lock().await;
            if let Some((previous, mut run)) =
                persistence::load(worker, &record.assignment.index.id).await?
            {
                if !run.phase.terminal() {
                    run.phase = RunPhase::Blocked;
                    run.error = Some(format!("brain activation: {error:#}"));
                    run.revision += 1;
                    persistence::save(
                        worker,
                        Some(previous),
                        &run,
                        "activation_failed",
                        json!({"error":run.error}),
                    )
                    .await?;
                }
            }
        }
    }
    result
}

async fn run_inner(
    worker: &Worker,
    record: &Record,
    config: Config,
    cancel: CancellationToken,
) -> Result<(ExecutionStatus, Value)> {
    let id = &record.assignment.index.id;
    let gate = worker.lifecycle_gate(id).await;
    let context = {
        let _guard = gate.lock().await;
        let (previous, mut run) = match persistence::load(worker, id).await? {
            Some((record, run)) => (Some(record), run),
            None => (
                None,
                execution::initialize(
                    id,
                    serde_json::from_value(record.assignment.request.input.clone())?,
                    persistence::now(),
                )?,
            ),
        };
        if run.candidate_plan.is_some()
            || run.phase.terminal()
            || matches!(run.phase, RunPhase::Paused | RunPhase::Blocked)
        {
            return Ok(outcome(&run));
        }
        if run.phase == RunPhase::Cancelling {
            let ids: Vec<_> = run
                .instances
                .values()
                .filter(|i| i.status.active())
                .map(|i| i.id.clone())
                .collect();
            for id in ids {
                execution::prepare_cancel(&mut run, &id)?;
            }
            run.handled_revision = run.revision;
            persistence::save(worker, previous, &run, "cancellation_requested", json!({})).await?;
            return Ok(outcome(&run));
        }
        let context = execution::context(&mut run);
        run.handled_revision = run.revision;
        persistence::save(
            worker,
            previous,
            &run,
            "activation_started",
            json!({"observed_revision":context.revision}),
        )
        .await?;
        context
    };
    // A fresh, finite context is the only activation input. No session history
    // is resumed and no model waits for a child execution to finish.
    let decision =
        if context.plan.is_some() && context.ready.is_empty() && context.routes.is_empty() {
            Ok(ActivationDecision {
                run_id: context.run_id.clone(),
                activation: context.activation,
                control_epoch: context.control_epoch,
                reason: "Observation recorded; waiting for a legal wake source".into(),
                plan: None,
                dispatch: vec![],
                routes: vec![],
            })
        } else {
            container::activate(worker, &config, &context, cancel.clone()).await
        };
    if cancel.is_cancelled() {
        anyhow::bail!("brain activation interrupted");
    }
    let _guard = gate.lock().await;
    let (previous, mut run) = persistence::load(worker, id)
        .await?
        .context("brain state disappeared")?;
    if run.control_epoch != context.control_epoch || run.activation != context.activation {
        persistence::save(
            worker,
            Some(previous),
            &run,
            "activation_fenced",
            json!({"discarded_activation":context.activation}),
        )
        .await?;
        return Ok(outcome(&run));
    }
    match decision {
        Ok(decision) => {
            execution::decide(&run, &decision)?;
            anyhow::ensure!(
                context
                    .routes
                    .iter()
                    .all(|c| decision.routes.iter().any(|d| d.receipt == c.receipt)),
                "activation omitted a pending route decision"
            );
            if let Some(plan) = decision.plan {
                opencoder_brain::ontology::validate(&plan)?;
                run.candidate_plan = Some(PlanVersion {
                    id: format!("plan-{}", id.trim_start_matches("brain-")),
                    version: 1,
                    plan,
                    changelog: decision.reason.clone(),
                    tags: vec!["generated".into()],
                    confidence: Confidence::default(),
                    created_at: persistence::now(),
                    author: "brain".into(),
                });
            } else {
                // The finite plan determines readiness. The activation must
                // dispatch all ready work; it cannot silently omit a branch.
                for route in &decision.routes {
                    opencoder_brain::graph::apply(&mut run, route, persistence::now())?;
                    if run.phase == RunPhase::Blocked {
                        break;
                    }
                }
                let ready: Vec<_> = run
                    .instances
                    .values()
                    .filter(|i| i.status == StepStatus::Ready)
                    .map(|i| i.id.clone())
                    .collect();
                for instance_id in &ready {
                    if run.phase == RunPhase::Running
                        && run
                            .instances
                            .get(instance_id)
                            .is_some_and(|i| i.status == StepStatus::Ready)
                    {
                        execution::prepare(
                            &mut run,
                            instance_id,
                            &worker.inner.registration.id,
                            persistence::now(),
                        )?;
                    }
                }
            }
            persistence::save(
                worker,
                Some(previous),
                &run,
                "activation_completed",
                json!({"reason":decision.reason,"dispatch":decision.dispatch,"routes":decision.routes}),
            )
            .await?;
        }
        Err(error) => {
            run.phase = RunPhase::Blocked;
            run.error = Some(format!("brain activation: {error:#}"));
            run.revision += 1;
            persistence::save(
                worker,
                Some(previous),
                &run,
                "activation_failed",
                json!({"error":run.error}),
            )
            .await?;
        }
    }
    Ok(outcome(&run))
}

pub(super) fn outcome(run: &BrainRun) -> (ExecutionStatus, Value) {
    let status = match run.phase {
        RunPhase::Completed => ExecutionStatus::Done,
        RunPhase::Failed => ExecutionStatus::Error,
        RunPhase::Cancelled => ExecutionStatus::Cancelled,
        _ => ExecutionStatus::Idle,
    };
    (
        status,
        json!({"phase":run.phase,"plan":run.request.plan.as_ref().map(|p|PlanRef{id:p.id.clone(),version:p.version}),"deliverables":run.deliverables,"error":run.error}),
    )
}
