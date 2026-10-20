use super::*;

impl Managed {
    pub fn group_ready(&self, name: &str) -> bool {
        !self.groups.contains(name)
            || self.rows.get(&(name.into(), None)).is_none_or(|row| {
                !matches!(
                    row.phase,
                    Phase::Paused | Phase::Done | Phase::Error | Phase::Cancelled
                ) && row.pending_operation.is_none()
            })
    }
    pub fn group_finished(&mut self, name: &str, outcome: StepOutcome) -> Result<()> {
        if !self.groups.contains(name) {
            return Ok(());
        }
        let row = self.rows.get_mut(&(name.into(), None)).unwrap();
        row.phase = match outcome {
            StepOutcome::Done => Phase::Done,
            StepOutcome::Error => Phase::Error,
            StepOutcome::Cancelled => Phase::Cancelled,
        };
        row.updated_at = now_ms();
        self.save()
    }
    pub(super) fn group_command(
        &mut self,
        receipt: Receipt,
        active: &BTreeMap<Key, CancellationToken>,
    ) -> Result<Vec<StepDone>> {
        let command = &receipt.command;
        let key = (command.step.clone(), None);
        let members: Vec<_> = self
            .rows
            .keys()
            .filter(|(name, index)| name == &command.step && index.is_some())
            .cloned()
            .collect();
        if members.iter().any(|key| {
            self.rows[key]
                .pending_operation
                .as_ref()
                .is_some_and(|c| c.operation_id != command.operation_id)
        }) {
            control::finish(
                &self.root,
                &self.run,
                receipt,
                Some("an instance has another pending operation".into()),
            )?;
            return Ok(Vec::new());
        }
        if matches!(command.action, Action::Resume | Action::Retry) {
            for key in &members {
                let row = self.rows.get_mut(key).unwrap();
                if matches!(row.phase, Phase::Paused | Phase::WaitingRecovery) {
                    row.phase = Phase::Pending;
                    row.reason = Some(command.reason.clone());
                    row.updated_at = now_ms();
                }
            }
            let row = self.rows.get_mut(&key).unwrap();
            row.phase = if row.attempt == 0 {
                Phase::Pending
            } else {
                Phase::Running
            };
            if row.attempt > 0 {
                row.attempt += 1;
            }
            row.pending_operation = None;
            row.applied_operation = Some(command.operation_id.clone());
            row.updated_at = now_ms();
            self.save()?;
            control::finish(&self.root, &self.run, receipt, None)?;
            return Ok(Vec::new());
        }
        self.rows.get_mut(&key).unwrap().pending_operation = Some(command.clone());
        let mut completions = Vec::new();
        let mut stopping = false;
        if members.is_empty() && command.action == Action::Fail {
            self.save()?;
            return Ok(vec![StepDone {
                name: command.step.clone(),
                instance: None,
                started_at_ms: now_ms(),
                result: StepResult {
                    outcome: StepOutcome::Error,
                    error: Some(command.reason.clone()),
                    output_text: String::new(),
                    output_json: None,
                    session_id: None,
                },
            }]);
        }
        for key in members {
            let row = self.rows.get_mut(&key).unwrap();
            if matches!(row.phase, Phase::Done | Phase::Error | Phase::Cancelled) {
                continue;
            }
            if let Some(token) = active.get(&key) {
                row.pending_operation = Some(command.clone());
                stopping = true;
                // Save the command before cancelling its executor.
                self.save()?;
                token.cancel();
                continue;
            }
            if command.action == Action::Pause {
                row.phase = Phase::Paused;
                row.reason = Some(command.reason.clone());
                row.updated_at = now_ms();
            } else {
                control::archive(&self.root, &self.run, &row.step, row.instance, row.attempt)?;
                row.pending_operation = Some(command.clone());
                stopping = true;
                completions.push(StepDone {
                    name: row.step.clone(),
                    instance: row.instance,
                    started_at_ms: now_ms(),
                    result: StepResult {
                        outcome: StepOutcome::Error,
                        error: Some(command.reason.clone()),
                        output_text: String::new(),
                        output_json: None,
                        session_id: None,
                    },
                });
            }
        }
        if !stopping {
            let row = self.rows.get_mut(&key).unwrap();
            row.phase = if command.action == Action::Pause {
                Phase::Paused
            } else {
                Phase::Error
            };
            row.pending_operation = None;
            row.applied_operation = Some(command.operation_id.clone());
            row.updated_at = now_ms();
        }
        self.save()?;
        if !stopping {
            control::finish(&self.root, &self.run, receipt, None)?;
        }
        Ok(completions)
    }
}
