use super::FleetStore;
use anyhow::{ensure, Result};
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use serde_json::Value;

impl FleetStore {
    /// Count admission-blocking work in one snapshot without materializing and
    /// sorting every historical execution for each readiness request.
    pub async fn active_execution_count(&self) -> Result<u64> {
        let _guard = self.gate.lock().await;
        let mut rows = self
            .conn
            .query(
                "SELECT kind,status,COUNT(*),\
                 SUM(typeof(id)<>'text' OR typeof(created_at)<>'integer' \
                     OR typeof(kind)<>'text' OR typeof(node_id)<>'text' \
                     OR typeof(status)<>'text') \
                 FROM execution_index GROUP BY kind,status",
                (),
            )
            .await?;
        let mut count = 0;
        while let Some(row) = rows.next().await? {
            // The old paged read decoded every index, including inactive work.
            // Keep invalid stored records visible instead of silently omitting
            // an unknown kind/status or a wrongly typed field from the count.
            ensure!(
                row.get::<i64>(3)? == 0,
                "invalid execution index field type"
            );
            let _: ExecutionKind = serde_json::from_value(Value::String(row.get(0)?))?;
            let status: ExecutionStatus = serde_json::from_value(Value::String(row.get(1)?))?;
            if matches!(
                status,
                ExecutionStatus::Pending
                    | ExecutionStatus::Running
                    | ExecutionStatus::Idle
                    | ExecutionStatus::Cancelling
            ) {
                count += u64::try_from(row.get::<i64>(2)?)?;
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests;
