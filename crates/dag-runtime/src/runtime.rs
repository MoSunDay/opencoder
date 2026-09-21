//! Whole-run scheduling loop for one claimed DAG run: validate the spec
//! snapshot, schedule ready steps on a bounded-concurrency [`JoinSet`],
//! write per-step artifacts, stream events upstream, honor cancellation,
//! and fold the terminal status (`run_outcome`: cancelled > error > done).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use opencoder_core::message::now_ms;
use opencoder_dag::artifacts::{run_root, validate_run_id, validate_step_slug};
use opencoder_dag::protocol::DagClaimedRun;
use opencoder_dag::{
    run_outcome, validate, DagRunStatus, DagSpec, DagStatusReport, StepKind, StepOutcome,
    StepStates,
};
use opencoder_node::uplink::Uplink;
use opencoder_store::SessionMeta;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::dag_events::{run_finished_event, run_started_event, RunEventSink};
use crate::exec::{execute_agent_step, ExecDeps, StepCtx, StepResult};
mod dynamic;
mod scheduler;

/// Everything the run loop needs besides the claimed run itself.
pub struct RunDeps {
    /// Bearer-authenticated uplink for event batches + the terminal status report.
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
    pub(crate) instance: Option<usize>,
    pub(crate) started_at_ms: i64,
    pub(crate) result: StepResult,
}

/// Execute one claimed run to a terminal status and report it upstream.
///
/// Defensive by design: the server already validated the spec at dispatch,
/// but a corrupted/edited snapshot still folds into a clean `error` report
/// instead of wedging the worker. Status delivery retries once, then returns
/// the transport/persistence error to the execution owner.
pub async fn execute_run(
    deps: RunDeps,
    run: DagClaimedRun,
    cancel_rx: watch::Receiver<bool>,
) -> Result<DagRunStatus> {
    execute_run_inner(deps, run, cancel_rx, false).await
}

/// Explicit same-node recovery skips only successfully persisted step checkpoints.
pub async fn resume_run(
    deps: RunDeps,
    run: DagClaimedRun,
    cancel_rx: watch::Receiver<bool>,
) -> Result<DagRunStatus> {
    execute_run_inner(deps, run, cancel_rx, true).await
}
async fn execute_run_inner(
    deps: RunDeps,
    run: DagClaimedRun,
    cancel_rx: watch::Receiver<bool>,
    resume: bool,
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
    ensure_run_session(&exec.store, &run).await;
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

    let (states, step_errors, user_cancelled) = match scheduler::schedule(
        exec,
        &deps.workflow_root,
        &run,
        &sink,
        cancel_rx,
        resume,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            return fail_run(
                &deps.uplink,
                run,
                sink,
                format!("DAG scheduling failed: {error:#}"),
            )
            .await
        }
    };
    let terminal = if user_cancelled {
        DagRunStatus::Cancelled
    } else {
        fold_terminal(&run.spec, &states)
    };

    let error_text = run_error_text(&run.spec, &states, &step_errors);
    sink.emit(run_finished_event(terminal.as_str(), error_text.as_deref()));
    if let Err(error) = sink.close().await {
        warn!(run_id = %run.run_id, error = %error, "dag event uploader did not flush cleanly");
    }
    report_status(&deps.uplink, &run.run_id, terminal, error_text.clone()).await?;
    info!(run_id = %run.run_id, status = %terminal, "dag run finished");
    Ok(terminal)
}

/// Guarantee the run's own session row exists before any step mirrors output
/// onto it: `session_events.session_id` is a foreign key, so a missing row
/// would silently drop every `step_output` record this run produces. The
/// node's DAG workload normally creates the session first (with its own
/// title/agent) and the store's insert is `OR IGNORE`, so this is a no-op
/// there and a safety net everywhere else (resume of an older run, direct
/// runtime callers, tests). Failing to create it only warns — the run's own
/// event/status delivery does not depend on the node store.
async fn ensure_run_session(store: &Arc<dyn opencoder_store::Store>, run: &DagClaimedRun) {
    let now = now_ms();
    let meta = SessionMeta {
        id: run.run_id.clone(),
        title: Some(run.spec.name.clone()),
        created_at: now,
        updated_at: now,
        ..Default::default()
    };
    if let Err(error) = store.create_session(&meta).await {
        warn!(
            run_id = %run.run_id,
            error = %error,
            "run session unavailable; step_output rows for this run may not persist"
        );
    }
}

/// Dispatch one step by kind, wrapped in its per-step wall-clock budget.
/// A timeout cancels the step token and folds to `Error("step timeout")`.
async fn execute_step(ctx: &StepCtx, exec: &ExecDeps, cancel: CancellationToken) -> StepResult {
    // Wasm owns its budget and cancellation so process/container cleanup
    // completes before the runtime publishes the step's terminal status.
    if matches!(&ctx.step.kind, StepKind::Wasm { .. }) {
        // Mirror the step's output into the node store as it is produced
        // (`step_output` events on the run's session). The tail batch is
        // flushed BEFORE the result is returned, so the console never sees a
        // terminal step whose last output is still buffered — on success,
        // timeout, cancellation, and error alike.
        let output = crate::step_log::StepOutputLog::for_instance(
            exec.store.clone(),
            &ctx.run_id,
            &ctx.step.name,
            ctx.instance,
        );
        let result =
            crate::exec::wasm::execute_wasm_step_logged(ctx, cancel, Some(output.clone())).await;
        output.close().await;
        return result;
    }
    let fut = async {
        match &ctx.step.kind {
            StepKind::Agent { .. } => execute_agent_step(ctx, exec, cancel.clone()).await,
            StepKind::Wasm { .. } | StepKind::Dynamic { .. } => {
                unreachable!("only executable steps are dispatched")
            }
        }
    };
    tokio::pin!(fut);
    match ctx.step.timeout_secs {
        None => fut.await,
        Some(secs) => tokio::select! {
            result = &mut fut => result,
            _ = tokio::time::sleep(Duration::from_secs(secs)) => {
                cancel.cancel();
                // Reap the executor and flush its logs before publishing a timeout.
                let mut result = fut.await;
                result.outcome = StepOutcome::Error;
                result.error = Some(format!("step timeout after {secs}s"));
                result
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

/// Retry one transient failure, then surface the delivery error to the owner.
async fn report_status(
    uplink: &Uplink,
    run_id: &str,
    status: DagRunStatus,
    error: Option<String>,
) -> Result<()> {
    let report = DagStatusReport {
        run_id: run_id.to_string(),
        status: status.as_str().to_string(),
        error,
    };
    for attempt in 0..2 {
        match uplink.dag_status(&report).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                warn!(run_id, attempt, error = %error, "dag status report failed");
                if attempt == 1 {
                    return Err(
                        error.context("DAG terminal status delivery failed after 2 attempts")
                    );
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
    unreachable!("the second status attempt always returns")
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
    if let Err(close_error) = sink.close().await {
        warn!(run_id = %run.run_id, error = %close_error, "dag event uploader did not flush cleanly");
    }
    report_status(uplink, &run.run_id, DagRunStatus::Error, Some(error)).await?;
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
