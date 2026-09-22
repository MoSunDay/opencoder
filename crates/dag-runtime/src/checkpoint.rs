use anyhow::Result;
use opencoder_dag::{DagClaimedRun, StepOutcome, StepOutputs, StepStates};
use std::{io::Write, path::Path};

pub(crate) fn restore(
    root: &Path,
    run: &DagClaimedRun,
    states: &mut StepStates,
    outputs: &mut StepOutputs,
) -> Result<()> {
    for step in &run.spec.steps {
        let dir = opencoder_dag::artifacts::step_dir(root, &run.run_id, &step.name)
            .map_err(anyhow::Error::msg)?;
        let meta = dir.join("meta.json");
        if !meta.exists() {
            continue;
        }
        let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(meta)?)?;
        let collected_failure = meta["outcome"] == "error" && matches!(
            step.kind, opencoder_dag::StepKind::Dynamic {
                failure_policy: opencoder_dag::FailurePolicy::CollectAll, ..
            }
        );
        if meta["outcome"] != "done" && !collected_failure {
            continue;
        }
        let output = serde_json::from_slice(&std::fs::read(dir.join("output.json"))?)?;
        states.insert(step.name.clone(), if collected_failure { StepOutcome::Error } else { StepOutcome::Done });
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
