//! Small detail views never retain frozen runtime configuration or queue data.
use super::{cache::Cache, Record};
use crate::lifecycle::TodoInitialization;
use opencoder_core::fleet::{CreateExecution, ExecutionIndex};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Error, ErrorKind, Write};

const MAX_VIEW_BYTES: usize = 8 * 1024;

/// Read-only fields for Inspect. This type cannot be saved as an execution.
#[derive(Deserialize)]
pub(crate) struct Inspection {
    pub index: ExecutionIndex,
    pub request: CreateExecution,
    pub definition: Option<Value>,
    pub annotations: Value,
    pub result: Value,
    pub error: Option<String>,
    pub todo_initialization: Option<TodoInitialization>,
}

impl Inspection {
    pub(super) fn cache() -> Cache {
        // Together with the 48 MiB body cache, payload retention stays 64 MiB.
        Cache::new(16 * 1024 * 1024, 32 * 1024)
    }

    pub(super) fn encode(record: &Record) -> Option<Vec<u8>> {
        #[derive(Serialize)]
        struct View<'a> {
            index: &'a ExecutionIndex,
            request: &'a CreateExecution,
            definition: &'a Option<Value>,
            annotations: &'a Value,
            result: &'a Value,
            error: &'a Option<String>,
            todo_initialization: Option<TodoInitialization>,
        }
        let view = View {
            index: &record.assignment.index,
            request: &record.assignment.request,
            definition: &record.assignment.definition,
            annotations: &record.annotations,
            result: &record.result,
            error: &record.error,
            todo_initialization: record.lifecycle.todo_initialization,
        };
        let mut bytes = Limited(Vec::new());
        serde_json::to_writer(&mut bytes, &view).ok()?;
        Some(bytes.0)
    }

    pub(super) fn from_record(record: Record) -> Self {
        Self {
            index: record.assignment.index,
            request: record.assignment.request,
            definition: record.assignment.definition,
            annotations: record.annotations,
            result: record.result,
            error: record.error,
            todo_initialization: record.lifecycle.todo_initialization,
        }
    }
}

struct Limited(Vec<u8>);

impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_VIEW_BYTES {
            return Err(Error::new(ErrorKind::FileTooLarge, "detail cache limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../tests/journal_memory/inspection.rs"]
mod tests;
