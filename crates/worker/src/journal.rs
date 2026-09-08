use crate::{
    layout::ALL_KINDS,
    lifecycle::{self, Lifecycle, StopIntent},
    DirectoryLayout,
};
use anyhow::{bail, Result};
use opencoder_core::{fleet::*, SseEvt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

mod io;
#[cfg(test)]
use io::normalize_legacy_index;
use io::{durable_json, validate_record};
pub(crate) use io::{read_record, same_execution};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Record {
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
    pub records: BTreeMap<String, Record>,
}

impl Journal {
    pub fn open(layout: DirectoryLayout) -> Result<Self> {
        let mut journal = Self {
            layout,
            locations: BTreeMap::new(),
            records: BTreeMap::new(),
        };
        journal.load_legacy()?;
        journal.load_current()?;
        let ids: Vec<_> = journal.records.keys().cloned().collect();
        for id in ids {
            let mut record = journal.records[&id].clone();
            let transition =
                lifecycle::recover(record.assignment.index.status, record.lifecycle.stop_intent);
            if transition.status != record.assignment.index.status
                || transition.intent != record.lifecycle.stop_intent
            {
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
                journal.save(record)?;
            }
        }
        Ok(journal)
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
        durable_json(&path, &record)?;
        self.locations.insert(id.clone(), location);
        self.records.insert(id, record);
        Ok(())
    }

    pub fn uses_legacy(&self, id: &str) -> bool {
        self.locations.get(id) == Some(&RecordLocation::Legacy)
    }

    pub fn begin_run(&mut self, id: &str, result: Value) -> Result<()> {
        let mut record = self
            .records
            .get(id)
            .cloned()
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

    pub fn mark_todo_initialized(&mut self, id: &str) -> Result<()> {
        let mut record = self
            .records
            .get(id)
            .cloned()
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

    pub fn request_stop(
        &mut self,
        id: &str,
        intent: StopIntent,
        active: bool,
    ) -> Result<StopUpdate> {
        let mut record = self
            .records
            .get(id)
            .cloned()
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

    pub fn finalize(
        &mut self,
        id: &str,
        proposed: ExecutionStatus,
        result: Value,
        error: Option<String>,
    ) -> Result<ExecutionStatus> {
        let mut record = self
            .records
            .get(id)
            .cloned()
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

    fn load_legacy(&mut self) -> Result<()> {
        let root = self.layout.legacy_records_root();
        if !root.exists() {
            return Ok(());
        }
        if std::fs::symlink_metadata(&root)?.file_type().is_symlink() {
            bail!("legacy journal root cannot be a symlink");
        }
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                bail!(
                    "journal records cannot be symlinks: {}",
                    entry.path().display()
                );
            }
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let record = read_record(&path, true)?;
            if path.file_stem().and_then(|value| value.to_str())
                != Some(record.assignment.index.id.as_str())
            {
                bail!("legacy journal file name does not match execution id");
            }
            self.insert_loaded(record, RecordLocation::Legacy)?;
        }
        Ok(())
    }

    fn load_current(&mut self) -> Result<()> {
        for kind in ALL_KINDS {
            let root = self.layout.kind_root(kind);
            if !root.exists() {
                continue;
            }
            if std::fs::symlink_metadata(&root)?.file_type().is_symlink() {
                bail!("execution kind root cannot be a symlink");
            }
            for entry in std::fs::read_dir(root)? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                if entry.file_type()?.is_symlink() {
                    bail!(
                        "execution directories cannot be symlinks: {}",
                        entry.path().display()
                    );
                }
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let id = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("non-UTF8 execution directory"))?;
                let path = self.layout.record_path(kind, &id)?;
                if !path.exists() {
                    continue;
                }
                let record = read_record(&path, false)?;
                validate_record(&record, Some(kind))?;
                if id != record.assignment.index.id {
                    bail!("execution directory name does not match execution id");
                }
                self.insert_loaded(record, RecordLocation::Current)?;
            }
        }
        Ok(())
    }

    fn insert_loaded(&mut self, record: Record, location: RecordLocation) -> Result<()> {
        let id = record.assignment.index.id.clone();
        if let Some(existing) = self.records.get(&id) {
            let existing_location = self.locations[&id];
            if !coexisting_records_are_valid(
                &self.layout,
                existing,
                existing_location,
                &record,
                location,
            )? {
                bail!("conflicting legacy and current execution records for {id}");
            }
            if existing_location == RecordLocation::Current {
                return Ok(());
            }
        }
        self.locations.insert(id.clone(), location);
        self.records.insert(id, record);
        Ok(())
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

fn coexisting_records_are_valid(
    layout: &DirectoryLayout,
    left: &Record,
    left_location: RecordLocation,
    right: &Record,
    right_location: RecordLocation,
) -> Result<bool> {
    if serde_json::to_value(left)? == serde_json::to_value(right)? {
        return Ok(true);
    }
    let (legacy, current) = match (left_location, right_location) {
        (RecordLocation::Legacy, RecordLocation::Current) => (left, right),
        (RecordLocation::Current, RecordLocation::Legacy) => (right, left),
        _ => return Ok(false),
    };
    Ok(same_execution(legacy, current)
        && crate::migration::receipt_matches_legacy(
            layout,
            &legacy.assignment.index.id,
            legacy.assignment.index.kind,
        )?)
}

#[cfg(test)]
#[path = "journal/tests.rs"]
mod tests;
