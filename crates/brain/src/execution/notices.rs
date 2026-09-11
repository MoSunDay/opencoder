use anyhow::{ensure, Context, Result};
use opencoder_core::brain::*;

pub fn apply_notice(run: &mut BrainRun, notice: &BrainNotice) -> Result<bool> {
    ensure!(
        notice.parent.run_id == run.id,
        "notice belongs to another root"
    );
    let cursor_key = format!(
        "{}:{}:{}",
        notice.node_id, notice.execution.id, notice.parent.attempt
    );
    if run
        .source_cursors
        .get(&cursor_key)
        .is_some_and(|seq| *seq >= notice.sequence)
    {
        return Ok(false);
    }
    let instance = run
        .instances
        .get_mut(&notice.parent.instance_id)
        .context("unknown step instance")?;
    // Late receipts from an earlier attempt are acknowledged but cannot
    // overwrite the current attempt, or revive a completed instance.
    if notice.parent.attempt != instance.attempt || instance.status.terminal() {
        return Ok(false);
    }
    ensure!(
        instance.execution.as_ref() == Some(&notice.execution),
        "notice execution mismatch"
    );
    ensure!(
        instance
            .node_id
            .as_ref()
            .is_none_or(|id| *id == notice.node_id),
        "notice owner mismatch"
    );
    ensure!(
        notice.status.active()
            || matches!(
                notice.status,
                StepStatus::Succeeded | StepStatus::Failed | StepStatus::Cancelled
            ),
        "invalid child status"
    );
    if instance.status == StepStatus::Running && notice.status == StepStatus::Queued {
        return Ok(false);
    }
    instance.node_id = Some(notice.node_id.clone());
    instance.status = notice.status;
    instance.reason = notice.error.clone().unwrap_or_default();
    if notice.status == StepStatus::Succeeded {
        let step = run
            .request
            .plan
            .as_ref()
            .unwrap()
            .plan
            .steps
            .iter()
            .find(|s| s.id == instance.step_id)
            .unwrap();
        match notice.output.as_ref() {
            Some(output) => match crate::ontology::accepts(&step.output, &output.value) {
                Ok(()) => instance.output = Some(output.clone()),
                Err(e) => {
                    instance.status = StepStatus::Failed;
                    instance.reason = format!("output contract: {e}");
                }
            },
            None => {
                instance.status = StepStatus::Failed;
                instance.reason = "execution completed without an output envelope".into();
            }
        }
    }
    if instance.status.terminal() {
        instance.finished_at = Some(notice.at_ms);
        for receipt in run
            .actions
            .values_mut()
            .filter(|a| a.instance_id == instance.id && a.attempt == instance.attempt)
        {
            receipt.state = ReceiptState::Settled;
        }
        // Only a definitive failed receipt can trigger a new attempt.
        let step = run
            .request
            .plan
            .as_ref()
            .unwrap()
            .plan
            .steps
            .iter()
            .find(|s| s.id == instance.step_id)
            .unwrap();
        if instance.status == StepStatus::Failed
            && instance.attempt < step.action.max_attempts
            && run.phase != RunPhase::Cancelling
        {
            instance.status = StepStatus::Ready;
            instance.execution = None;
            instance.node_id = None;
            instance.finished_at = None;
        }
    }
    run.source_cursors.insert(cursor_key, notice.sequence);
    run.revision += 1;
    super::advance(run, notice.at_ms)?;
    Ok(true)
}
