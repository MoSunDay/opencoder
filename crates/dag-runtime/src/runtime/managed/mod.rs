//! Nonterminal operator holds are deliberately absent from StepStates, so
//! dependents (including all_done) cannot consume an unresolved attempt.
use crate::{control, exec::StepResult, runtime::StepDone};
use anyhow::Result;
use opencoder_core::message::now_ms;
use opencoder_dag::{
    control::{Action, Phase, Receipt, StepStatus},
    DagClaimedRun, RecoveryPolicy, StepKind, StepOutcome,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

pub(super) type Key = (String, Option<usize>);

pub(super) struct Managed {
    root: PathBuf,
    run: String,
    rows: BTreeMap<Key, StepStatus>,
    enabled: bool,
    groups: BTreeSet<String>,
}

impl Managed {
    pub fn open(root: &Path, run: &DagClaimedRun, resume: bool) -> Result<Self> {
        let enabled = run
            .spec
            .steps
            .iter()
            .any(|s| s.recovery_policy == RecoveryPolicy::Operator);
        let rows = if enabled && resume {
            control::status(root, &run.run_id)?
                .into_iter()
                .map(|mut r| {
                    if r.phase == Phase::Running {
                        r.phase = Phase::WaitingRecovery;
                        r.reason = Some("executor interrupted; inspect before resuming".into());
                    }
                    ((r.step.clone(), r.instance), r)
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        let mut this = Self {
            root: root.into(),
            run: run.run_id.clone(),
            rows,
            enabled,
            groups: run
                .spec
                .steps
                .iter()
                .filter(|s| {
                    s.recovery_policy == RecoveryPolicy::Operator
                        && matches!(s.kind, StepKind::Dynamic { .. })
                })
                .map(|s| s.name.clone())
                .collect(),
        };
        for step in &run.spec.steps {
            if step.recovery_policy == RecoveryPolicy::Operator {
                this.register(&step.name, None);
            }
        }
        if resume {
            this.restore_completed()?;
        }
        this.save()?;
        Ok(this)
    }
    pub fn register(&mut self, name: &str, instance: Option<usize>) {
        self.rows
            .entry((name.into(), instance))
            .or_insert_with(|| StepStatus {
                step: name.into(),
                instance,
                attempt: 0,
                phase: Phase::Pending,
                reason: None,
                updated_at: now_ms(),
                pending_operation: None,
                applied_operation: None,
            });
    }
    pub fn ready(&self, name: &str, index: Option<usize>) -> bool {
        if index.is_some() && !self.group_ready(name) {
            return false;
        }
        self.rows
            .get(&(name.into(), index))
            .is_none_or(|r| r.phase == Phase::Pending)
    }
    pub fn waiting(&self) -> bool {
        self.rows
            .values()
            .any(|r| matches!(r.phase, Phase::Paused | Phase::WaitingRecovery))
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn instances(&mut self, name: &str, count: usize) -> Result<()> {
        if let Some(row) = self.rows.get_mut(&(name.into(), None)) {
            if row.attempt == 0 {
                row.attempt = 1;
                row.phase = Phase::Running;
            }
        }
        for index in 0..count {
            self.register(name, Some(index));
        }
        self.save()
    }
    pub fn begin(&mut self, name: &str, index: Option<usize>) -> Result<()> {
        if let Some(row) = self.rows.get_mut(&(name.into(), index)) {
            control::archive(&self.root, &self.run, name, index, row.attempt)?;
            row.attempt += 1;
            row.phase = Phase::Running;
            row.reason = None;
            row.updated_at = now_ms();
            self.save()?;
        }
        Ok(())
    }
    fn save(&self) -> Result<()> {
        if self.enabled {
            control::save(
                &self.root,
                &self.run,
                &self.rows.values().cloned().collect::<Vec<_>>(),
            )?;
        }
        Ok(())
    }
}

mod commands;
mod completion;
mod groups;
mod reconcile;

#[cfg(test)]
mod tests;
