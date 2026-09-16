use serde::{Deserialize, Serialize};

/// The run was submitted to the execution plane (`execution_id` is set).
pub const SCHEDULE_RUN_FIRED: &str = "fired";
/// The tick was too old to catch up on (only the most recent missed tick
/// fires; older ones are recorded for the audit trail and skipped).
pub const SCHEDULE_RUN_MISSED: &str = "missed";
/// Submission failed (`error` carries the reason; retriable on later ticks).
pub const SCHEDULE_RUN_ERROR: &str = "error";

/// One row of the `schedule_runs` ledger: a cron control-plane scheduler
/// fire for one schedule id. Rows are keyed `(schedule_id, scheduled_for_ms)`
/// so re-firing the same tick is idempotent. Terminal execution state lives
/// in the execution index — the scheduler reads it from there, the Store
/// only persists the ledger. Pure data: scheduling lives above the Store
/// (see `libsql_store/schedule.rs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRunRecord {
    /// `ScheduleJob::id` from `schedules.json`.
    pub schedule_id: String,
    /// Schedule kind (`brain` | `team` | `todos` | `agent` | `dag`).
    pub kind: String,
    /// Target of the fire (plan-def id / definition id / template path / ...).
    pub target: String,
    /// Unix-ms of the cron tick this row is for (the PK's time half).
    pub scheduled_for_ms: i64,
    /// Unix-ms of when the scheduler actually processed the tick.
    pub fired_at_ms: i64,
    /// Deterministic execution id `<kind>-<schedule_id>-<scheduled_for_ms>`;
    /// `None` for skipped ticks.
    pub execution_id: Option<String>,
    /// [`SCHEDULE_RUN_FIRED`] | [`SCHEDULE_RUN_MISSED`] | [`SCHEDULE_RUN_ERROR`].
    pub status: String,
    /// Failure reason when `status` is [`SCHEDULE_RUN_ERROR`].
    pub error: Option<String>,
    /// Convenience boolean for skipped ticks (`status == missed`).
    pub missed: bool,
}
