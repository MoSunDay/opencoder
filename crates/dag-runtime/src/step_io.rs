//! Per-step artifact bookkeeping extracted from the run loop
//! ([`crate::runtime`]): persisting a finished step's artifacts
//! (`output.txt` / `output.json` / `meta.json`), folding its result into
//! the scheduling state + `step_done` event, and giving steps that never
//! ran a terminal outcome. Pure IO helpers — no scheduling decisions.

use std::collections::BTreeMap;

use anyhow::Context;
use opencoder_core::message::now_ms;
use opencoder_dag::artifacts::{meta_value, output_snapshot, step_dir};
use opencoder_dag::protocol::DagClaimedRun;
use opencoder_dag::{DagSpec, StepOutcome, StepOutputs, StepStates};
use serde_json::Value;
use tracing::warn;

use crate::dag_events::{step_done_event, RunEventSink};
use crate::exec::StepResult;
use crate::runtime::StepDone;

/// Persist one finished step's artifacts + state + `step_done` event.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn record_step(
    d: StepDone,
    states: &mut StepStates,
    outputs: &mut StepOutputs,
    step_errors: &mut BTreeMap<String, String>,
    workflow_root: &std::path::Path,
    run: &DagClaimedRun,
    sink: &RunEventSink,
) {
    let StepDone {
        name,
        started_at_ms,
        result,
    } = d;
    let outcome = result.outcome;
    if let Some(err) = &result.error {
        step_errors.insert(name.clone(), err.clone());
    }
    write_step_artifacts(workflow_root, run, &name, started_at_ms, &result).await;
    states.insert(name.clone(), outcome);
    outputs.insert(
        name.clone(),
        result.output_json.clone().unwrap_or(Value::Null),
    );
    sink.emit(step_done_event(
        &name,
        outcome.is_success(),
        result.error.as_deref(),
        output_snapshot(&result.output_text),
    ));
}

/// Write `<step>/output.txt`, optional `output.json`, and `meta.json`.
/// Artifact IO failures degrade the step to `Error` (reported, not fatal to
/// the process) — but scheduling state still records the executor's own
/// outcome; the run-level fold surfaces the write failure via step_errors.
pub(crate) async fn write_step_artifacts(
    workflow_root: &std::path::Path,
    run: &DagClaimedRun,
    name: &str,
    started_at_ms: i64,
    result: &StepResult,
) {
    let dir = match step_dir(workflow_root, &run.run_id, name) {
        Ok(d) => d,
        Err(e) => {
            warn!(run_id = %run.run_id, step = %name, error = %e, "step dir rejected");
            return;
        }
    };
    let wrote = async {
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("{}", dir.display()))?;
        if !result.output_text.is_empty() {
            tokio::fs::write(dir.join("output.txt"), &result.output_text).await?;
        }
        if let Some(json) = &result.output_json {
            tokio::fs::write(
                dir.join("output.json"),
                serde_json::to_string_pretty(json).unwrap_or_default(),
            )
            .await?;
        }
        let meta = meta_value(
            name,
            outcome_str(&result.outcome),
            started_at_ms,
            now_ms(),
            result.error.as_deref(),
        );
        tokio::fs::write(
            dir.join("meta.json"),
            serde_json::to_string(&meta).unwrap_or_default(),
        )
        .await?;
        Ok::<(), anyhow::Error>(())
    };
    if let Err(e) = wrote.await {
        warn!(run_id = %run.run_id, step = %name, error = %e, "step artifact write failed");
    }
}

/// Give every step that never ran (cancelled run, or transitively blocked by
/// a failed upstream) a terminal outcome, a `meta.json`, and a `step_done`
/// frame so the event projection never shows a dangling step.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn mark_unfinished(
    spec: &DagSpec,
    states: &mut StepStates,
    step_errors: &mut BTreeMap<String, String>,
    workflow_root: &std::path::Path,
    run: &DagClaimedRun,
    sink: &RunEventSink,
    reason: &str,
    outcome: StepOutcome,
) {
    for step in &spec.steps {
        if states.contains_key(&step.name) {
            continue;
        }
        states.insert(step.name.clone(), outcome);
        step_errors.insert(step.name.clone(), reason.to_string());
        let result = StepResult {
            outcome,
            error: Some(reason.to_string()),
            output_text: String::new(),
            output_json: None,
            session_id: None,
        };
        write_step_artifacts(workflow_root, run, &step.name, now_ms(), &result).await;
        sink.emit(step_done_event(
            &step.name,
            outcome.is_success(),
            Some(reason),
            String::new(),
        ));
    }
}

/// Stable wire string for a step outcome (`StepOutcome` itself carries no
/// serializer — the artifacts contract is owned here).
fn outcome_str(o: &StepOutcome) -> &'static str {
    match o {
        StepOutcome::Done => "done",
        StepOutcome::Error => "error",
        StepOutcome::Cancelled => "cancelled",
    }
}
