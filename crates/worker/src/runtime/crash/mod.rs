//! Durable worker-exit provenance and terminal Brain settlement.
use crate::{brain::v4::state, journal::Journal};
use anyhow::{Context, Result};
use opencoder_core::{brain::layered::*, fleet::ExecutionKind, message::now_ms};
use opencoder_store::Store;
use serde_json::json;
use std::path::Path;

mod offline;
mod proof;
pub use offline::settle_brain_crash;

#[derive(Debug)]
pub(crate) struct ProcessCrash(pub String);
impl std::fmt::Display for ProcessCrash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Brain decision process crashed: {}", self.0)
    }
}
impl std::error::Error for ProcessCrash {}

const MARKER: &str = "worker-lifecycle.json";
pub(crate) const REASON: &str = "Worker process crashed; Brain failed; start a new Brain run";

pub(crate) fn unclean(data_dir: &Path) -> Result<bool> {
    let path = data_dir.join(MARKER);
    crate::migration_io::reject_symlink(&path, "worker lifecycle")?;
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    match value["state"].as_str() {
        Some("running") => Ok(true),
        Some("clean") => Ok(false),
        _ => anyhow::bail!("invalid worker lifecycle marker"),
    }
}

pub(crate) fn mark(data_dir: &Path, clean: bool) -> Result<()> {
    let path = data_dir.join(MARKER);
    crate::migration_io::reject_symlink(&path, "worker lifecycle")?;
    crate::migration_io::durable_json(
        &path,
        &json!({
            "state":if clean {"clean"} else {"running"},"at_ms":now_ms()
        }),
    )
}

/// Store failure precedes the journal and any capacity write. Retrying after
/// either durable write is harmless and never resets decision markers.
pub(crate) async fn fail_root(
    store: &dyn Store,
    journal: &mut Journal,
    id: &str,
    reason: &str,
) -> Result<LayeredSnapshot> {
    let record = journal.records.get(id).context("Brain journal missing")?;
    anyhow::ensure!(
        record.assignment.index.kind == ExecutionKind::Brain,
        "not a Brain root"
    );
    let snapshot = match store.brain_layered(id).await? {
        Some(snapshot) => snapshot,
        None => {
            let mut initial =
                opencoder_brain::layered::initialize(id, &state::request(record)?, now_ms())?;
            initial.run.phase = LayeredPhase::Failed;
            initial.run.error = Some(reason.into());
            initial.events.push(opencoder_brain::layered::event(
                &initial.run,
                "run_failed",
                Some(reason.into()),
            ));
            store.commit_brain_layered(&initial).await?
        }
    };
    let snapshot = match opencoder_brain::layered::fail(&snapshot, reason.into(), now_ms()) {
        Some(change) => store.commit_brain_layered(&change).await?,
        None => snapshot,
    };
    let (status, result) = state::outcome(&snapshot);
    // The projection is authoritative. A previous graceful interrupt cannot
    // turn a later crash into a resumable journal record.
    if let Some(record) = journal.records.get(id) {
        if !record.assignment.index.status.terminal() && record.lifecycle.stop_intent.is_some() {
            let mut record = record.clone();
            record.lifecycle.stop_intent = None;
            journal.save(record)?;
        }
    }
    journal.finalize(id, status, result, snapshot.run.error.clone())?;
    Ok(snapshot)
}

pub(crate) async fn recover_failed(
    store: &dyn Store,
    journal: &mut Journal,
    data_dir: &Path,
) -> Result<()> {
    if unclean(data_dir)? {
        let ids: Vec<_> = journal
            .records
            .values()
            .filter(|record| {
                record.assignment.index.kind == ExecutionKind::Brain
                    && !record.assignment.index.status.terminal()
            })
            .map(|record| record.assignment.index.id.clone())
            .collect();
        for id in ids {
            fail_root(store, journal, &id, REASON).await?;
        }
    }
    Ok(())
}
