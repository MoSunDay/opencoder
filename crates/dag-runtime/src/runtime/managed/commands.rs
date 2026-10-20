use super::*;

impl Managed {
    pub fn poll(
        &mut self,
        active: &BTreeMap<Key, CancellationToken>,
        cancelled: bool,
    ) -> Result<Vec<StepDone>> {
        let mut completions = Vec::new();
        if !self.enabled {
            return Ok(completions);
        }
        for receipt in control::pending(&self.root, &self.run)? {
            let command = &receipt.command;
            let key = (command.step.clone(), command.instance);
            let error = match self.rows.get(&key) {
                _ if cancelled => Some("run cancellation takes precedence"),
                None => Some("step instance not initialized"),
                Some(row) if row.applied_operation.as_deref() == Some(&command.operation_id) => {
                    control::finish(&self.root, &self.run, receipt, None)?;
                    continue;
                }
                Some(row) if row.attempt != command.expected_attempt => {
                    Some("step attempt changed")
                }
                Some(row) if matches!(row.phase, Phase::Done | Phase::Error | Phase::Cancelled) => {
                    Some("terminal step cannot be changed")
                }
                Some(row)
                    if row
                        .pending_operation
                        .as_ref()
                        .is_some_and(|old| old.operation_id != command.operation_id) =>
                {
                    Some("another operation is stopping this attempt")
                }
                Some(row)
                    if matches!(command.action, Action::Resume | Action::Retry)
                        && !matches!(row.phase, Phase::Paused | Phase::WaitingRecovery) =>
                {
                    Some("resume or retry requires a stopped unresolved attempt")
                }
                _ => None,
            };
            if let Some(error) = error {
                control::finish(&self.root, &self.run, receipt, Some(error.into()))?;
                continue;
            }
            if command.instance.is_none() && self.groups.contains(&command.step) {
                completions.extend(self.group_command(receipt, active)?);
                continue;
            }
            if let Some(token) = active.get(&key) {
                self.rows.get_mut(&key).unwrap().pending_operation = Some(command.clone());
                self.save()?;
                token.cancel();
                continue;
            }
            if command.action == Action::Fail {
                control::archive(
                    &self.root,
                    &self.run,
                    &command.step,
                    command.instance,
                    command.expected_attempt,
                )?;
                self.rows.get_mut(&key).unwrap().pending_operation = Some(command.clone());
                self.save()?;
                completions.push(StepDone {
                    name: command.step.clone(),
                    instance: command.instance,
                    started_at_ms: now_ms(),
                    result: StepResult {
                        outcome: StepOutcome::Error,
                        error: Some(command.reason.clone()),
                        output_text: String::new(),
                        output_json: None,
                        session_id: None,
                    },
                });
            } else {
                let row = self.rows.get_mut(&key).unwrap();
                row.phase = if command.action == Action::Pause {
                    Phase::Paused
                } else {
                    Phase::Pending
                };
                row.reason = Some(command.reason.clone());
                row.updated_at = now_ms();
                row.pending_operation = None;
                row.applied_operation = Some(command.operation_id.clone());
                self.save()?;
                control::finish(&self.root, &self.run, receipt, None)?;
            }
        }
        Ok(completions)
    }
}
