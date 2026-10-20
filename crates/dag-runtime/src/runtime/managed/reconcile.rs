use super::*;

impl Managed {
    pub(super) fn restore_completed(&mut self) -> Result<()> {
        for row in self.rows.values_mut() {
            let dir = opencoder_dag::artifacts::execution_dir(
                &self.root,
                &self.run,
                &row.step,
                row.instance,
            )
            .map_err(anyhow::Error::msg)?;
            let raw = match std::fs::read(dir.join("meta.json")) {
                Ok(raw) => raw,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let meta: serde_json::Value = serde_json::from_slice(&raw)?;
            // The result is persisted before the control snapshot. Recover that
            // commit gap without launching an already completed executor.
            if meta["outcome"] == "done"
                && matches!(row.phase, Phase::Running | Phase::WaitingRecovery)
            {
                row.phase = Phase::Done;
                row.reason = None;
                row.updated_at = now_ms();
            }
        }
        Ok(())
    }

    pub fn cancel_pending(
        &mut self,
        active: &BTreeMap<Key, CancellationToken>,
        run: &DagClaimedRun,
    ) -> Result<()> {
        for (key, row) in &mut self.rows {
            let finalizer = run.spec.steps.iter().any(|s| {
                s.name == row.step && s.trigger_rule == opencoder_dag::TriggerRule::AllDone
            });
            if (finalizer && matches!(row.phase, Phase::Pending | Phase::Running))
                || active.contains_key(key)
                || matches!(row.phase, Phase::Done | Phase::Error | Phase::Cancelled)
            {
                continue;
            }
            row.phase = Phase::Cancelled;
            row.reason = Some("run cancelled".into());
            row.pending_operation = None;
            row.updated_at = now_ms();
        }
        self.save()
    }

    pub fn is_cancelled(&self, name: &str) -> bool {
        self.rows
            .get(&(name.into(), None))
            .is_some_and(|row| row.phase == Phase::Cancelled)
    }

    pub fn finish(&mut self, states: &opencoder_dag::StepStates) -> Result<()> {
        for (name, outcome) in states {
            if let Some(row) = self.rows.get_mut(&(name.clone(), None)) {
                row.phase = match outcome {
                    StepOutcome::Done => Phase::Done,
                    StepOutcome::Error => Phase::Error,
                    StepOutcome::Cancelled => Phase::Cancelled,
                };
                row.updated_at = now_ms();
            }
        }
        self.save()
    }
}
