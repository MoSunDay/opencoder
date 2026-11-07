use super::FleetStore;
use crate::rdb_store::tx;
use anyhow::Result;
use opencoder_core::fleet::ExecutionStatus;

/// Count admission-blocking work in one snapshot without materializing and
/// sorting every historical execution for each readiness request. Rows that
/// fail to decode (unknown kind/status or wrong field type) stay visible as
/// errors instead of silently draining the count.
pub(super) fn active_execution_count(store: &rdb::store::Store) -> Result<u64> {
    let mut count = 0u64;
    for (_, value) in tx::scan_prefix(store, b"xrow/")? {
        let record: opencoder_core::fleet::ExecutionIndex = crate::rdb_store::kv::decode(&value)?;
        if matches!(
            record.status,
            ExecutionStatus::Pending
                | ExecutionStatus::Running
                | ExecutionStatus::Idle
                | ExecutionStatus::Cancelling
        ) {
            count += 1;
        }
    }
    Ok(count)
}

#[path = "access/counts.rs"]
mod database_access;
