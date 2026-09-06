use anyhow::Result;
use std::path::Path;

pub(crate) fn migrate_layout(data_dir: &Path, workflow_root: Option<&Path>) -> Result<()> {
    let report = opencoder_worker::migrate_layout(data_dir, workflow_root)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
