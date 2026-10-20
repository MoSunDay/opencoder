use anyhow::Result;
use opencoder_dag::{DagClaimedRun, StepOutcome, StepOutputs, StepStates};
use std::{io::Write, path::Path};

pub(crate) fn restore(
    root: &Path,
    run: &DagClaimedRun,
    states: &mut StepStates,
    outputs: &mut StepOutputs,
) -> Result<()> {
    let controls = crate::control::status(root, &run.run_id)?;
    for step in &run.spec.steps {
        let dir = opencoder_dag::artifacts::step_dir(root, &run.run_id, &step.name)
            .map_err(anyhow::Error::msg)?;
        let meta = dir.join("meta.json");
        if !meta.exists() {
            continue;
        }
        let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(meta)?)?;
        let managed = step.recovery_policy == opencoder_dag::RecoveryPolicy::Operator;
        let preserved = controls
            .iter()
            .find(|row| row.step == step.name && row.instance.is_none())
            .and_then(|row| match row.phase {
                opencoder_dag::control::Phase::Error => Some(StepOutcome::Error),
                opencoder_dag::control::Phase::Cancelled => Some(StepOutcome::Cancelled),
                _ => None,
            });
        let collected_failure = !managed
            && meta["outcome"] == "error"
            && matches!(
                step.kind,
                opencoder_dag::StepKind::Dynamic {
                    failure_policy: opencoder_dag::FailurePolicy::CollectAll,
                    ..
                }
            );
        if meta["outcome"] != "done" && !collected_failure && preserved.is_none() {
            continue;
        }
        let output = serde_json::from_slice(&std::fs::read(dir.join("output.json"))?)?;
        states.insert(
            step.name.clone(),
            if let Some(outcome) = preserved {
                outcome
            } else if collected_failure {
                StepOutcome::Error
            } else {
                StepOutcome::Done
            },
        );
        outputs.insert(step.name.clone(), output);
    }
    Ok(())
}
pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(temp, path)?;
    std::fs::File::open(path.parent().unwrap())?.sync_all()?;
    Ok(())
}
