//! Durable expansion manifests and group bookkeeping, separate from definitions.
use crate::{dag_events::RunEventSink, exec::StepResult};
use anyhow::{Context, Result};
use opencoder_dag::{dynamic, DagClaimedRun, StepKind, StepOutcome, StepOutputs, StepSpec};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::Path};
use tokio_util::sync::CancellationToken;

pub(super) struct Group {
    pub items: Vec<Value>,
    pub outcomes: Vec<Option<StepOutcome>>,
    pub outputs: Vec<Value>,
    pub running: BTreeSet<usize>,
    pub token: CancellationToken,
    pub error: Option<String>,
    pub started: i64,
    pub collect_all: bool,
}

pub(super) fn open(
    root: &Path,
    run: &DagClaimedRun,
    step: &StepSpec,
    input: &Value,
    outputs: &StepOutputs,
    token: CancellationToken,
    resume: bool,
) -> Result<Group> {
    let StepKind::Dynamic { source, template, failure_policy } = &step.kind else {
        unreachable!()
    };
    let dir = opencoder_dag::artifacts::step_dir(root, &run.run_id, &step.name)
        .map_err(anyhow::Error::msg)?;
    std::fs::create_dir_all(&dir)?;
    let manifest = dir.join("instances.json");
    let items = if manifest.exists() {
        let saved: Value = serde_json::from_slice(&std::fs::read(&manifest)?)?;
        dynamic::validate_items(template, &saved).map_err(anyhow::Error::msg)?
    } else {
        let items =
            dynamic::expand(source, template, input, outputs).map_err(anyhow::Error::msg)?;
        crate::checkpoint::write(&manifest, &serde_json::to_vec(&items)?)?;
        items
    };
    let mut group = Group {
        collect_all: *failure_policy == opencoder_dag::FailurePolicy::CollectAll,
        outcomes: vec![None; items.len()],
        outputs: vec![Value::Null; items.len()],
        items,
        running: BTreeSet::new(),
        token,
        error: None,
        started: opencoder_core::message::now_ms(),
    };
    if resume {
        for i in 0..group.items.len() {
            let dir = dir.join("instances").join(i.to_string());
            let path = dir.join("meta.json");
            if !path.exists() {
                continue;
            }
            let meta: Value = serde_json::from_slice(&std::fs::read(path)?)?;
            let saved_outcome = match meta["outcome"].as_str() {
                Some("done") => Some(StepOutcome::Done),
                Some("error") if group.collect_all => Some(StepOutcome::Error),
                _ => None,
            };
            if let Some(outcome) = saved_outcome {
                group.outcomes[i] = Some(outcome);
                if outcome == StepOutcome::Error {
                    group.error.get_or_insert_with(|| format!("instance {i}: preserved failure"));
                }
                group.outputs[i] =
                    serde_json::from_slice(&std::fs::read(dir.join("output.json"))?)?;
            }
        }
        for i in 0..group.items.len() {
            if group.outcomes[i].is_none() {
                reset(root, &run.run_id, &step.name, Some(i), None)?;
            }
        }
    }
    Ok(group)
}

impl Group {
    pub fn next(&self) -> Option<usize> {
        if self.token.is_cancelled() {
            return None;
        }
        self.outcomes
            .iter()
            .enumerate()
            .find_map(|(i, s)| (s.is_none() && !self.running.contains(&i)).then_some(i))
    }
    pub fn result(&self) -> Option<StepResult> {
        let outcome = dynamic::group_outcome(&self.outcomes)?;
        Some(StepResult {
            outcome,
            error: self.error.clone(),
            output_text: String::new(),
            output_json: Some(Value::Array(self.outputs.clone())),
            session_id: None,
        })
    }
    pub fn progress(&self, root: &Path, run: &str, step: &str, sink: &RunEventSink) -> Result<()> {
        let at = opencoder_core::message::now_ms();
        let payload =
            json!({"instances":dynamic::progress(&self.outcomes, self.running.len()),"at_ms":at});
        let dir =
            opencoder_dag::artifacts::step_dir(root, run, step).map_err(anyhow::Error::msg)?;
        crate::checkpoint::write(&dir.join("progress.json"), &serde_json::to_vec(&payload)?)?;
        sink.emit(opencoder_dag::DagEventIn {
            kind: "step_progress".into(),
            step: Some(step.into()),
            payload,
            at_ms: at,
        });
        Ok(())
    }
    pub async fn cancel_pending(
        &mut self,
        root: &Path,
        run: &DagClaimedRun,
        name: &str,
    ) -> Result<()> {
        for i in 0..self.items.len() {
            if self.outcomes[i].is_some() || self.running.contains(&i) {
                continue;
            }
            let result = StepResult {
                outcome: StepOutcome::Cancelled,
                error: Some(self.error.clone().unwrap_or_else(|| "run cancelled".into())),
                output_text: String::new(),
                output_json: None,
                session_id: None,
            };
            crate::step_io::write_execution_artifacts(
                root,
                &run.run_id,
                name,
                Some(i),
                self.started,
                &result,
            )
            .await
            .context("persist cancelled instance")?;
            self.outcomes[i] = Some(StepOutcome::Cancelled);
        }
        Ok(())
    }
}

/// Replace old receipts before a retry so readers cannot observe old terminal state.
pub(super) fn start(
    root: &Path,
    run: &str,
    step: &str,
    index: Option<usize>,
    at: i64,
) -> Result<()> {
    reset(root, run, step, index, Some(at))
}

fn reset(root: &Path, run: &str, step: &str, index: Option<usize>, at: Option<i64>) -> Result<()> {
    let dir = opencoder_dag::artifacts::execution_dir(root, run, step, index)
        .map_err(anyhow::Error::msg)?;
    std::fs::create_dir_all(&dir)?;
    for name in [
        "output.json",
        "output.txt",
        "transcript.txt",
        "events.ndjson",
    ] {
        match std::fs::remove_file(dir.join(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    crate::checkpoint::write(&dir.join("session.json"), b"{}")?;
    crate::checkpoint::write(
        &dir.join("meta.json"),
        &serde_json::to_vec(&json!({
            "step":step, "index":index, "outcome":if at.is_some() {"running"} else {"pending"}, "started_at_ms":at
        }))?,
    )
}
