use super::*;

impl Managed {
    /// Returns true when completion is held, keeping dependencies unresolved.
    pub async fn completed(&mut self, done: &mut StepDone, cancelled: bool) -> Result<bool> {
        let key = (done.name.clone(), done.instance);
        let Some(row) = self.rows.get_mut(&key) else {
            return Ok(false);
        };
        let pending = row.pending_operation.take();
        let mut rejected = None;
        let phase = if done.result.outcome == StepOutcome::Done {
            if pending.is_some() {
                rejected = Some("step finished before stop took effect".into());
            }
            Phase::Done
        } else if cancelled {
            done.result.outcome = StepOutcome::Cancelled;
            Phase::Cancelled
        } else if pending.as_ref().is_some_and(|c| c.action == Action::Fail) {
            done.result.outcome = StepOutcome::Error;
            done.result.error = pending.as_ref().map(|c| c.reason.clone());
            Phase::Error
        } else if pending.as_ref().is_some_and(|c| c.action == Action::Pause) {
            Phase::Paused
        } else {
            Phase::WaitingRecovery
        };
        crate::step_io::write_execution_artifacts(
            &self.root,
            &self.run,
            &done.name,
            done.instance,
            done.started_at_ms,
            &done.result,
        )
        .await?;
        let held = matches!(phase, Phase::Paused | Phase::WaitingRecovery);
        row.phase = phase;
        row.reason = done.result.error.clone();
        row.updated_at = now_ms();
        if let Some(command) = &pending {
            row.applied_operation = Some(command.operation_id.clone());
        }
        if held {
            let dir = opencoder_dag::artifacts::execution_dir(
                &self.root,
                &self.run,
                &done.name,
                done.instance,
            )
            .map_err(anyhow::Error::msg)?;
            let mut meta: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.join("meta.json"))?)?;
            meta["result_outcome"] = serde_json::to_value(done.result.outcome)?;
            meta["outcome"] = serde_json::to_value(&row.phase)?;
            crate::checkpoint::write(&dir.join("meta.json"), &serde_json::to_vec(&meta)?)?;
        }
        if done.instance.is_some() && held {
            if let Some(group) = self.rows.get_mut(&(done.name.clone(), None)) {
                if group.pending_operation.is_none() {
                    group.phase = Phase::WaitingRecovery;
                    group.updated_at = now_ms();
                }
            }
        }
        self.save()?;
        if let Some(command) = pending {
            if command.instance != done.instance {
                return Ok(held);
            }
            if cancelled {
                rejected = Some("run cancelled".into());
            }
            control::finish(
                &self.root,
                &self.run,
                Receipt {
                    command,
                    phase: String::new(),
                    error: None,
                    updated_at: now_ms(),
                },
                rejected,
            )?;
        }
        Ok(held)
    }
}
