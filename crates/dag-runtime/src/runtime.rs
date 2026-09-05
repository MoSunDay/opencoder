//! Whole-run scheduling loop for one claimed DAG run: validate the spec
//! snapshot, schedule ready steps on a bounded-concurrency [`JoinSet`],
//! write per-step artifacts, stream events upstream, honor cancellation,
//! and fold the terminal status (`run_outcome`: cancelled > error > done).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use opencoder_core::message::now_ms;
use opencoder_dag::artifacts::{run_root, validate_run_id, validate_step_slug};
use opencoder_dag::protocol::DagClaimedRun;
use opencoder_dag::{
    ready_steps, run_outcome, validate, DagRunStatus, DagSpec, DagStatusReport, SandboxMode,
    StepKind, StepOutcome, StepOutputs, StepStates,
};
use opencoder_node::uplink::Uplink;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::dag_events::{run_finished_event, run_started_event, step_started_event, RunEventSink};
use crate::exec::{execute_agent_step, execute_python_step, ExecDeps, StepCtx, StepResult};
use crate::step_io::{mark_unfinished, record_step};

/// Upper bound on simultaneously executing steps; excess ready steps stay
/// queued and are recomputed each round (fairness by spec order).
pub const MAX_CONCURRENT_STEPS: usize = 4;

/// Everything the run loop needs besides the claimed run itself.
pub struct RunDeps {
    /// Signed uplink for event batches + the terminal status report.
    pub uplink: Arc<Uplink>,
    /// Shared per-step executor dependencies (store/client/workdir/config).
    pub exec: ExecDeps,
    /// Artifact root: `<workflow_root>/<run_id>/<step>/...`.
    pub workflow_root: PathBuf,
}

/// One spawned step's completion payload (consumed by the run loop and by
/// [`crate::step_io::record_step`]).
pub(crate) struct StepDone {
    pub(crate) name: String,
    pub(crate) started_at_ms: i64,
    pub(crate) result: StepResult,
}

/// Execute one claimed run to a terminal status and report it upstream.
///
/// Defensive by design: the server already validated the spec at dispatch,
/// but a corrupted/edited snapshot still folds into a clean `error` report
/// instead of wedging the worker. Status-post failures retry once then warn
/// (the server's lost-run sweep converges the row eventually).
pub async fn execute_run(
    deps: RunDeps,
    run: DagClaimedRun,
    cancel_rx: watch::Receiver<bool>,
) -> Result<DagRunStatus> {
    let sink = RunEventSink::new(Arc::clone(&deps.uplink), run.run_id.clone());
    let exec = Arc::new(deps.exec);

    if let Err(errs) = validate(&run.spec) {
        let error = format!("invalid spec snapshot: {}", errs.join("; "));
        return fail_run(&deps.uplink, run, sink, error).await;
    }
    if !validate_run_id(&run.run_id) {
        let error = format!("illegal run id {:?}", run.run_id);
        return fail_run(&deps.uplink, run, sink, error).await;
    }
    if run.spec.steps.iter().any(|s| !validate_step_slug(&s.name)) {
        // validate() already covers this; kept as a belt-and-braces guard
        // before any path is built from a step name.
        let error = "invalid step slug in spec snapshot".to_string();
        return fail_run(&deps.uplink, run, sink, error).await;
    }
    if let Err(e) = tokio::fs::create_dir_all(
        run_root(&deps.workflow_root, &run.run_id).expect("run id validated above"),
    )
    .await
    {
        let error = format!("create run root: {e:#}");
        return fail_run(&deps.uplink, run, sink, error).await;
    }

    info!(
        run_id = %run.run_id,
        dag_id = %run.dag_id,
        steps = run.spec.steps.len(),
        "dag run executing"
    );
    sink.emit(run_started_event(&run.spec.name));

    // Run-level cancel token: the watched flag flips (heartbeat cancel /
    // shutdown), the forwarder cancels it, every in-flight child token
    // fires, executors converge through their own interrupt paths.
    let run_cancel = CancellationToken::new();
    {
        let token = run_cancel.clone();
        let mut rx = cancel_rx.clone();
        tokio::spawn(async move {
            await_flag(&mut rx).await;
            token.cancel();
        });
    }

    let mut states: StepStates = BTreeMap::new();
    let mut outputs: StepOutputs = BTreeMap::new();
    let mut step_errors: BTreeMap<String, String> = BTreeMap::new();
    let mut inflight: JoinSet<StepDone> = JoinSet::new();
    // Steps currently executing. `ready_steps` only excludes steps whose
    // outcome already landed in `states`, so this set fills that blind
    // spot: without it, every completion would re-dispatch the step's
    // still-running siblings (duplicate sessions, events, artifacts).
    let mut inflight_names: BTreeSet<String> = BTreeSet::new();
    let mut cancel_rx = cancel_rx;

    let terminal = loop {
        if *cancel_rx.borrow_and_update() {
            run_cancel.cancel();
            // Drain in-flight steps through `record_step` — the same path
            // the main loop uses — so a cancelled run keeps their artifacts
            // and `step_done` frames; a state-only fold would lose both.
            // Each task converges quickly once its child token fired; the
            // executor's own outcome (usually `Cancelled`) wins.
            while let Some(res) = inflight.join_next().await {
                match res {
                    Ok(d) => {
                        inflight_names.remove(&d.name);
                        record_step(
                            d,
                            &mut states,
                            &mut outputs,
                            &mut step_errors,
                            &deps.workflow_root,
                            &run,
                            &sink,
                        )
                        .await;
                    }
                    Err(e) => warn!(error = %e, "dag step task failed during cancel drain"),
                }
            }
            mark_unfinished(
                &run.spec,
                &mut states,
                &mut step_errors,
                &deps.workflow_root,
                &run,
                &sink,
                "run cancelled",
                StepOutcome::Cancelled,
            )
            .await;
            break fold_terminal(&run.spec, &states);
        }

        // Spawn ready steps while concurrency budget remains; excess ready
        // steps stay queued (recomputed next round, so no starvation).
        // Steps already in flight are filtered out: `ready_steps` cannot
        // see them (no outcome in `states` yet) and would re-dispatch them.
        // Collected first so the filter's borrow of the set ends here.
        let ready: Vec<String> = ready_steps(&run.spec, &states)
            .into_iter()
            .filter(|n| !inflight_names.contains(n))
            .collect();
        for name in ready {
            if inflight.len() >= MAX_CONCURRENT_STEPS {
                break;
            }
            let Some(step) = run.spec.steps.iter().find(|s| s.name == name) else {
                continue;
            };
            inflight_names.insert(name.clone());
            let ctx = StepCtx {
                run_id: run.run_id.clone(),
                spec: run.spec.clone(),
                step: step.clone(),
                states: states.clone(),
                outputs: outputs.clone(),
                workflow_root: deps.workflow_root.clone(),
            };
            sink.emit(step_started_event(&name));
            let exec = Arc::clone(&exec);
            let token = run_cancel.child_token();
            let started = now_ms();
            let task_name = name.clone();
            inflight.spawn(async move {
                // A panicked executor must not take the run (or the worker)
                // down: fold it into an Error outcome with the step's name.
                let result = match futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(
                    execute_step(&ctx, &exec, token),
                ))
                .await
                {
                    Ok(r) => r,
                    Err(_) => StepResult {
                        outcome: StepOutcome::Error,
                        error: Some("step executor panicked".into()),
                        output_text: String::new(),
                        output_json: None,
                        session_id: None,
                    },
                };
                StepDone {
                    name: task_name,
                    started_at_ms: started,
                    result,
                }
            });
        }

        if inflight.is_empty() {
            match run_outcome(&run.spec, &states) {
                Some(status) => break status,
                None => {
                    // Nothing runnable, nothing in flight, run not terminal:
                    // remaining steps are transitively blocked behind a
                    // failed upstream — fold them as errors and finish.
                    mark_unfinished(
                        &run.spec,
                        &mut states,
                        &mut step_errors,
                        &deps.workflow_root,
                        &run,
                        &sink,
                        "blocked: upstream step did not succeed",
                        StepOutcome::Error,
                    )
                    .await;
                    break fold_terminal(&run.spec, &states);
                }
            }
        }

        // A fresh await_flag future per round keeps the receiver reusable.
        tokio::select! {
            biased;
            _ = await_flag(&mut cancel_rx) => { /* loop head handles it */ }
            done = inflight.join_next() => {
                // A completed step must not be re-dispatched before its
                // outcome lands in `states`: `ready_steps` only consults
                // `states`, so the inflight set covers that blind spot.
                match done {
                    Some(Ok(d)) => {
                        inflight_names.remove(&d.name);
                        record_step(d, &mut states, &mut outputs, &mut step_errors, &deps.workflow_root, &run, &sink).await;
                    }
                    Some(Err(e)) => warn!(error = %e, "dag step task failed"),
                    None => {}
                }
            }
        }
    };

    let error_text = run_error_text(&run.spec, &states, &step_errors);
    sink.emit(run_finished_event(terminal.as_str(), error_text.as_deref()));
    sink.close().await;
    report_status(&deps.uplink, &run.run_id, terminal, error_text.clone()).await;
    info!(run_id = %run.run_id, status = %terminal, "dag run finished");
    Ok(terminal)
}

/// Dispatch one step by kind, wrapped in its per-step wall-clock budget.
/// A timeout cancels the step token and folds to `Error("step timeout")`.
async fn execute_step(ctx: &StepCtx, exec: &ExecDeps, cancel: CancellationToken) -> StepResult {
    // runc python steps own their timeout: the sandbox driver enforces its
    // own budget and its KILL + `delete --force` cleanup tail must run
    // after a timeout — an outer `tokio::time::timeout` would drop the
    // future first and swallow the cleanup, leaking the container. The
    // in-process VM (threads that cannot be interrupted) and agent steps
    // keep the outer budget: folding them to Error on timeout is the
    // documented behavior.
    if matches!(
        &ctx.step.kind,
        StepKind::Python {
            sandbox: Some(SandboxMode::Runc),
            ..
        }
    ) {
        return execute_python_step(ctx).await;
    }
    let fut = async {
        match &ctx.step.kind {
            StepKind::Agent { .. } => execute_agent_step(ctx, exec, cancel.clone()).await,
            StepKind::Python { .. } => execute_python_step(ctx).await,
        }
    };
    match ctx.step.timeout_secs {
        None => fut.await,
        Some(secs) => match tokio::time::timeout(Duration::from_secs(secs), fut).await {
            Ok(res) => res,
            Err(_) => {
                cancel.cancel();
                StepResult {
                    outcome: StepOutcome::Error,
                    error: Some(format!("step timeout after {secs}s")),
                    output_text: String::new(),
                    output_json: None,
                    session_id: None,
                }
            }
        },
    }
}

/// Terminal fold with a defensive default (every step has an outcome by the
/// time this runs, so `run_outcome` is always `Some`).
fn fold_terminal(spec: &DagSpec, states: &StepStates) -> DagRunStatus {
    run_outcome(spec, states).unwrap_or(DagRunStatus::Error)
}

/// Run-level error text: the first failed step's error, step-named so an
/// operator reading the run row knows where to look.
fn run_error_text(
    spec: &DagSpec,
    states: &StepStates,
    step_errors: &BTreeMap<String, String>,
) -> Option<String> {
    spec.steps
        .iter()
        .find(|s| states.get(&s.name) == Some(&StepOutcome::Error))
        .and_then(|s| {
            step_errors
                .get(&s.name)
                .map(|e| format!("step {}: {e}", s.name))
        })
}

/// Post the terminal status report: one retry, then warn (the server's
/// lost-run sweep converges the row eventually).
async fn report_status(uplink: &Uplink, run_id: &str, status: DagRunStatus, error: Option<String>) {
    let report = DagStatusReport {
        run_id: run_id.to_string(),
        status: status.as_str().to_string(),
        error,
    };
    for attempt in 0..2 {
        match uplink.dag_status(&report).await {
            Ok(()) => return,
            Err(e) => warn!(run_id, attempt, error = %e, "dag status report failed"),
        }
    }
}

/// Terminal-error path for runs that could not even start scheduling.
async fn fail_run(
    uplink: &Arc<Uplink>,
    run: DagClaimedRun,
    sink: RunEventSink,
    error: String,
) -> Result<DagRunStatus> {
    warn!(run_id = %run.run_id, error = %error, "dag run failed before scheduling");
    sink.emit(run_finished_event("error", Some(&error)));
    sink.close().await;
    report_status(uplink, &run.run_id, DagRunStatus::Error, Some(error)).await;
    Ok(DagRunStatus::Error)
}

/// Resolve once the watched boolean flag turns `true`. A dropped sender
/// parks forever instead of synthesizing a flip (mirrors the node crate's
/// semantics: cancellation must be explicit).
async fn await_flag(rx: &mut watch::Receiver<bool>) {
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
