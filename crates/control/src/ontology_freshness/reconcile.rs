//! One-shot settlement of freshness runs orphaned by a restart: runs left
//! queued or running when the previous process stopped.
use super::{
    orchestrator,
    planning::{apply, failed_outcome, poll_from_index, settle, PollOutcome},
};
use crate::AppState;
use opencoder_ontology::{
    AppError, FreshnessItemStatus, FreshnessOperation, FreshnessRun, FreshnessRunStatus,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

/// Module-local guard: reconciliation owns one delayed pass per process,
/// independent of the release lifecycle start flags.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Delay before the pass: placement, node reports and freshly triggered
/// drivers get a head start after a restart.
const START_DELAY: Duration = Duration::from_secs(15);
/// Live drivers hold the per-run request lock for their whole run; a short
/// acquire timeout means contended runs are skipped, not waited on.
const LOCK_WAIT: Duration = Duration::from_secs(1);

pub(crate) fn start(state: &Arc<AppState>) {
    if state.ontology.is_none() || STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        let lifecycle = match weak.upgrade() {
            Some(state) => state.lifecycle.clone(),
            None => return,
        };
        tokio::select! {
            _ = lifecycle.retired() => return,
            _ = tokio::time::sleep(START_DELAY) => {},
        }
        let Some(state) = weak.upgrade() else { return };
        if retiring(&state) {
            return;
        }
        if let Err(error) = pass(&state).await {
            tracing::warn!(%error, "Freshness reconciliation pass failed");
        }
    });
}

fn retiring(state: &AppState) -> bool {
    state.lifecycle.retiring.load(Ordering::SeqCst)
}

/// Visit every environment's active runs and settle what can be settled.
/// Per-run errors warn and continue with the next run or environment.
async fn pass(state: &Arc<AppState>) -> Result<(), AppError> {
    let Some(ontology) = state.ontology.as_ref() else {
        return Ok(());
    };
    let environments = ontology
        .freshness(String::new(), FreshnessOperation::Environments)
        .await?;
    for key in environments["items"].as_array().into_iter().flatten() {
        if retiring(state) {
            return Ok(());
        }
        let Some(env) = key["env_key"].as_str() else {
            continue;
        };
        let active = match ontology
            .freshness(env.to_owned(), FreshnessOperation::RunsActive)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(%error, env, "Freshness active run listing failed");
                continue;
            }
        };
        for record in active["items"].as_array().into_iter().flatten() {
            if retiring(state) {
                return Ok(());
            }
            match serde_json::from_value::<FreshnessRun>(record.clone()) {
                Ok(run) => {
                    if let Err(error) = settle_run(state, env, run).await {
                        tracing::warn!(%error, env, "Freshness run reconciliation failed");
                    }
                }
                Err(error) => tracing::warn!(%error, env, "Freshness run record is undecodable"),
            }
        }
    }
    Ok(())
}

/// Resolve one orphaned run. Items whose execution is terminal settle from
/// the fleet index and the result summary; pending items without an
/// execution failed at interruption. Runs whose executions are still live
/// stay active for a later pass instead of settling dishonestly.
async fn settle_run(state: &Arc<AppState>, env: &str, run: FreshnessRun) -> Result<(), AppError> {
    if retiring(state) {
        return Ok(());
    }
    let _guard = match tokio::time::timeout(
        LOCK_WAIT,
        state
            .fleet
            .request_lock(orchestrator::LOCK_SCOPE, &run.id.to_string()),
    )
    .await
    {
        Ok(Ok(guard)) => guard,
        Ok(Err(error)) => {
            tracing::warn!(%error, run = %run.id, "Freshness run lock unavailable");
            return Ok(());
        }
        Err(_) => {
            tracing::debug!(run = %run.id, "Freshness run owned by a live driver");
            return Ok(());
        }
    };
    let Some(mut fresh) = orchestrator::fetch(state, env, run.id).await else {
        return Ok(());
    };
    let mut in_flight = false;
    for item in fresh.items.iter_mut() {
        match (&item.execution_id, item.status) {
            (Some(execution), FreshnessItemStatus::Dispatched) => {
                match state.fleet.index(execution).await {
                    Ok(Some(index)) => match poll_from_index(index.status) {
                        PollOutcome::Pending => in_flight = true,
                        PollOutcome::Done => {
                            apply(item, orchestrator::summary_outcome(state, execution).await)
                        }
                        PollOutcome::Failed(outcome) => apply(item, outcome),
                    },
                    Ok(None) => {
                        apply(item, failed_outcome("execution record missing"))
                    }
                    Err(error) => {
                        tracing::warn!(%error, execution, "Freshness fleet index read failed");
                        in_flight = true;
                    }
                }
            }
            (None, FreshnessItemStatus::Pending) => {
                apply(item, failed_outcome("interrupted before dispatch"))
            }
            _ => {}
        }
    }
    if in_flight {
        tracing::debug!(run = %fresh.id, "Freshness run still in flight after restart");
        return Ok(());
    }
    let status = settle(&fresh.items);
    // Queued runs cannot jump straight to a terminal state; the data plane
    // requires the queued→running hop first.
    if fresh.status == FreshnessRunStatus::Queued {
        let Some(running) =
            orchestrator::save(state, env, &fresh, FreshnessRunStatus::Running).await
        else {
            return Err(AppError::dependency("freshness run cannot advance to running"));
        };
        fresh = running;
    }
    if orchestrator::save(state, env, &fresh, status)
        .await
        .is_none()
    {
        return Err(AppError::dependency("freshness run settlement save failed"));
    }
    Ok(())
}
