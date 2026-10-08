use anyhow::Result;
use std::path::Path;

pub(crate) fn migrate_layout(data_dir: &Path, workflow_root: Option<&Path>) -> Result<()> {
    let report = opencoder_worker::migrate_layout(data_dir, workflow_root)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

pub(crate) async fn settle_brain_crash(
    data_dir: &Path,
    workflow_root: Option<&Path>,
    run_id: &str,
    receipt_dir: &Path,
) -> Result<()> {
    let report =
        opencoder_worker::settle_brain_crash(data_dir, workflow_root, run_id, receipt_dir).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[tokio::test]
    async fn settlement_dispatch_refuses_foreign_execution_before_writing_receipts() {
        let directory = tempfile::tempdir().unwrap();
        let receipts = directory.path().join("receipts");
        let error = super::settle_brain_crash(directory.path(), None, "dag-foreign", &receipts)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("exact Brain run ID required"));
        assert!(!receipts.exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
