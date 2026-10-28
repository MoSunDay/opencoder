use super::{Record, RecordLocation};
use crate::lifecycle::StopIntent;
use opencoder_core::fleet::ExecutionIndex;

/// Resident scheduling metadata. Frozen requests and configuration stay on disk.
#[derive(Clone)]
pub(crate) struct Summary {
    pub index: ExecutionIndex,
    pub target: Option<String>,
    pub next_run_id: Option<String>,
    pub queue: Option<QueueSummary>,
    pub stop_intent: Option<StopIntent>,
    pub stopping_rerun: bool,
    #[cfg_attr(windows, allow(dead_code))]
    pub dag_parent: bool,
    pub schema_version: Option<u64>,
    pub(super) intent_generation: Option<u64>,
    pub(super) compatibility_error: Option<String>,
    pub(super) location: RecordLocation,
    pub(super) revision: u64,
}

#[derive(Clone)]
pub(crate) struct QueueSummary {
    pub ticket: Option<String>,
    pub sequence: u64,
}

impl Summary {
    pub(super) fn new(record: &Record, location: RecordLocation, revision: u64) -> Self {
        Self {
            index: record.assignment.index.clone(),
            target: record.assignment.request.target.clone(),
            next_run_id: record.result["next_run_id"].as_str().map(str::to_owned),
            queue: record.queue.as_ref().map(|queue| QueueSummary {
                ticket: queue.ticket.clone(),
                sequence: queue.sequence,
            }),
            stop_intent: record.lifecycle.stop_intent,
            stopping_rerun: record
                .lifecycle
                .todo_reruns
                .values()
                .any(|c| c.phase == "stopping"),
            dag_parent: record.annotations.get("dag_parent").is_some(),
            schema_version: record.assignment.request.input["schema_version"].as_u64(),
            intent_generation: record.annotations["layered_intent"]["generation"].as_u64(),
            compatibility_error: super::compatibility::validate(record)
                .err()
                .map(|error| error.to_string()),
            location,
            revision,
        }
    }
}
