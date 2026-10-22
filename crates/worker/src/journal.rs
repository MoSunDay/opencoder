use crate::{
    layout::ALL_KINDS,
    lifecycle::{self, Lifecycle, StopIntent},
    DirectoryLayout,
};
use anyhow::{bail, Result};
use opencoder_core::{fleet::*, SseEvt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

mod cache;
mod compatibility;
mod inspection;
mod io;
pub(crate) use inspection::Inspection;
mod loading;
mod projection;
mod reading;
mod reporting;
pub(crate) use reporting::DispatchCache;
mod summary;
#[cfg(test)]
use io::normalize_legacy_index;
use io::{durable_json, validate_record};
pub(crate) use io::{read_record, same_execution};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Record {
    #[serde(default)]
    pub annotations: Value,
    #[serde(default)]
    // Keep the full configuration snapshot out of every execution future's stack.
    pub queue: Option<Box<crate::operations::queue::QueuedRun>>,
    pub assignment: Assignment,
    pub result: Value,
    pub error: Option<String>,
    pub events: Vec<SseEvt>,
    #[serde(default)]
    pub lifecycle: Lifecycle,
}

pub(crate) struct StopUpdate {
    pub status: ExecutionStatus,
    pub signal: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordLocation {
    Current,
    Legacy,
}

pub(crate) struct Journal {
    layout: DirectoryLayout,
    locations: BTreeMap<String, RecordLocation>,
    // Report projections only; full records are read through get/read_record.
    records: BTreeMap<String, Record>,
    pub summaries: BTreeMap<String, summary::Summary>,
    revision: u64,
    cache: std::sync::Arc<std::sync::Mutex<cache::Cache>>,
    inspection_cache: std::sync::Arc<std::sync::Mutex<cache::Cache>>,
    read_slots: std::sync::Arc<tokio::sync::Semaphore>,
    report_candidates: BTreeSet<String>,
    terminal_candidates: BTreeSet<String>,
    pub inventory: opencoder_core::fleet::inventory::InventoryLog,
}

impl Journal {
    #[cfg(test)]
    pub fn open(layout: DirectoryLayout) -> Result<Self> {
        Self::load(layout)?.recover()
    }

    pub fn load(layout: DirectoryLayout) -> Result<Self> {
        let mut journal = Self {
            layout,
            locations: BTreeMap::new(),
            records: BTreeMap::new(),
            summaries: BTreeMap::new(),
            revision: 0,
            cache: Default::default(),
            inspection_cache: std::sync::Arc::new(std::sync::Mutex::new(Inspection::cache())),
            read_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
            report_candidates: BTreeSet::new(),
            terminal_candidates: BTreeSet::new(),
            inventory: Default::default(),
        };
        journal.load_legacy()?;
        journal.load_current()?;
        for summary in journal.summaries.values() {
            if let Some(error) = &summary.compatibility_error {
                bail!("{error}");
            }
        }
        *journal.cache.lock().unwrap() = cache::Cache::default();
        Ok(journal)
    }

    pub fn recover(mut self) -> Result<Self> {
        let ids: Vec<_> = self.summaries.keys().cloned().collect();
        for id in ids {
            let summary = &self.summaries[&id];
            if summary.index.status == ExecutionStatus::Pending
                && summary.queue.is_some()
                && summary.stop_intent.is_none()
            {
                continue;
            }
            let transition = lifecycle::recover(summary.index.status, summary.stop_intent);
            if transition.status != summary.index.status || transition.intent != summary.stop_intent
            {
                let mut record = self.get_sync(&id)?.expect("recovery summary exists");
                record.assignment.index.status = transition.status;
                record.lifecycle.stop_intent = transition.intent;
                record.error = match transition.status {
                    ExecutionStatus::Cancelled => {
                        Some("cancel completed during node recovery".into())
                    }
                    ExecutionStatus::Interrupted => {
                        Some("node process stopped; explicit resume required".into())
                    }
                    _ => record.error,
                };
                append_status_event(&mut record);
                self.save(record)?;
            }
        }
        Ok(self)
    }

    pub fn save(&mut self, record: Record) -> Result<()> {
        validate_record(&record, None)?;
        let id = record.assignment.index.id.clone();
        let location = self
            .locations
            .get(&id)
            .copied()
            .unwrap_or(RecordLocation::Current);
        let path = match location {
            RecordLocation::Current => {
                self.layout.record_path(record.assignment.index.kind, &id)?
            }
            RecordLocation::Legacy => self.layout.legacy_record_path(&id)?,
        };
        let inspection = durable_json(&path, &record)?
            .and_then(|stamp| Inspection::encode(&record).map(|bytes| (stamp, bytes)));
        self.locations.insert(id.clone(), location);
        self.update_report_candidate(&record);
        self.inventory.set(record.assignment.index.clone());
        self.install(record, location);
        if let Some((stamp, bytes)) = inspection {
            self.inspection_cache.lock().unwrap().insert(
                id,
                self.revision,
                stamp,
                std::sync::Arc::new(bytes),
            );
        }
        Ok(())
    }

    /// Drop an execution's journal record (and its record file) so full index
    /// reports stop resurrecting it after control-side deletion. Unknown ids
    /// return false; a missing record file is tolerated.
    pub fn forget(&mut self, id: &str) -> Result<bool> {
        let Some(record) = self.summaries.get(id) else {
            return Ok(false);
        };
        let location = self
            .locations
            .get(id)
            .copied()
            .unwrap_or(RecordLocation::Current);
        let path = match location {
            RecordLocation::Current => self.layout.record_path(record.index.kind, id)?,
            RecordLocation::Legacy => self.layout.legacy_record_path(id)?,
        };
        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(error.into());
            }
        }
        self.locations.remove(id);
        self.records.remove(id);
        self.summaries.remove(id);
        self.cache.lock().unwrap().remove(id);
        self.inspection_cache.lock().unwrap().remove(id);
        self.inventory.remove(id);
        self.terminal_candidates.remove(id);
        self.report_candidates.remove(id);
        Ok(true)
    }

    pub fn uses_legacy(&self, id: &str) -> bool {
        self.locations.get(id) == Some(&RecordLocation::Legacy)
    }

    pub async fn begin_run(&mut self, id: &str, result: Value) -> Result<()> {
        let mut record = self
            .get(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("execution not found"))?;
        if !lifecycle::can_start(record.assignment.index.status) {
            bail!(
                "execution cannot start from {}",
                record.assignment.index.status.as_str()
            );
        }
        record.assignment.index.status = ExecutionStatus::Running;
        record.lifecycle.stop_intent = None;
        record.result = result;
        record.error = None;
        append_status_event(&mut record);
        self.save(record)
    }

    pub async fn mark_todo_initialized(&mut self, id: &str) -> Result<()> {
        let mut record = self
            .get(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("execution not found"))?;
        if record.assignment.index.kind != ExecutionKind::Todos {
            bail!("only todo executions have workflow initialization state");
        }
        if record.lifecycle.todo_initialization
            == Some(crate::lifecycle::TodoInitialization::Initialized)
        {
            return Ok(());
        }
        record.lifecycle.todo_initialization =
            Some(crate::lifecycle::TodoInitialization::Initialized);
        self.save(record)
    }

    pub async fn request_stop(
        &mut self,
        id: &str,
        intent: StopIntent,
        active: bool,
    ) -> Result<StopUpdate> {
        let mut record = self
            .get(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("execution not found"))?;
        let current = record.assignment.index.status;
        let current_intent = record.lifecycle.stop_intent;
        let transition = lifecycle::request_stop(current, current_intent, intent, active);
        if transition.status != current || transition.intent != current_intent {
            record.assignment.index.status = transition.status;
            record.lifecycle.stop_intent = transition.intent;
            record.error = None;
            append_status_event(&mut record);
            self.save(record)?;
        }
        Ok(StopUpdate {
            status: transition.status,
            signal: active && !current.terminal(),
        })
    }

    pub async fn finalize(
        &mut self,
        id: &str,
        proposed: ExecutionStatus,
        result: Value,
        error: Option<String>,
    ) -> Result<ExecutionStatus> {
        let mut record = self
            .get(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("execution not found"))?;
        let current = record.assignment.index.status;
        let transition = lifecycle::finalize(current, record.lifecycle.stop_intent, proposed);
        if current.terminal() {
            return Ok(current);
        }
        record.assignment.index.status = transition.status;
        record.lifecycle.stop_intent = transition.intent;
        record.result = result;
        record.error = error;
        append_status_event(&mut record);
        self.save(record)?;
        Ok(transition.status)
    }
}

fn append_status_event(record: &mut Record) {
    let status = record.assignment.index.status;
    let seq = record
        .events
        .last()
        .and_then(|event| event.seq)
        .unwrap_or(0)
        + 1;
    record.events.push(SseEvt {
        kind: if status.terminal()
            || matches!(status, ExecutionStatus::Idle | ExecutionStatus::Interrupted)
        {
            "done"
        } else {
            "status"
        }
        .into(),
        data: json!({
            "status": status,
            "error": record.error,
            "stop_intent": record.lifecycle.stop_intent,
        }),
        ts: opencoder_core::message::now_ms(),
        seq: Some(seq),
    });
}

#[cfg(test)]
#[path = "journal/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/journal_memory/cases.rs"]
mod memory_tests;
