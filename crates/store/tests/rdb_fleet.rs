//! Behavioral tests for the rdb-backed FleetStore and TsRegistry. The suites
//! cover record ownership/cascades, brain plan history, report baselines,
//! capacity queueing, handoff receipts, frozen-assignment acceptance and the
//! tensor-registry contract.

#[path = "rdb_fleet/common.rs"]
pub(crate) mod common;
#[path = "rdb_fleet/report_support.rs"]
pub(crate) mod report_support;

#[path = "rdb_fleet/suite_brain.rs"]
mod suite_brain;
#[path = "rdb_fleet/suite_capacity.rs"]
mod suite_capacity;
#[path = "rdb_fleet/suite_handoff.rs"]
mod suite_handoff;
#[path = "rdb_fleet/suite_prepare.rs"]
mod suite_prepare;
#[path = "rdb_fleet/suite_records.rs"]
mod suite_records;
#[path = "rdb_fleet/suite_report.rs"]
mod suite_report;
#[path = "rdb_fleet/suite_report_fenced.rs"]
mod suite_report_fenced;
#[path = "rdb_fleet/suite_report_recovery.rs"]
mod suite_report_recovery;
#[path = "rdb_fleet/suite_ts.rs"]
mod suite_ts;
