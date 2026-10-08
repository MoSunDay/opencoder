//! Explicit, repeatable settlement of ONE stopped runtime's Brain root.
use super::{fail_root, proof, REASON};
use crate::{journal::Journal, DirectoryLayout, HostBinding};
use anyhow::{ensure, Context, Result};
use opencoder_core::{
    brain::layered::LayeredPhase,
    fleet::{valid_id, ExecutionKind},
};
use opencoder_store::{fleet::FleetStore, LibsqlStore, Store};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
struct Intent {
    data_dir: PathBuf,
    host_database: PathBuf,
    runtime_id: String,
    run_id: String,
    ticket: String,
    generation: Option<u64>,
    record_sha256: String,
    proof: proof::Proof,
}

pub async fn settle_brain_crash(
    data_dir: &Path,
    workflow_root: Option<&Path>,
    run_id: &str,
    receipt_dir: &Path,
) -> Result<serde_json::Value> {
    ensure!(
        cfg!(target_os = "linux"),
        "Brain crash settlement requires a Linux hosted Runtime"
    );
    ensure!(
        valid_id(run_id) && run_id.starts_with("brain-"),
        "exact Brain run ID required"
    );
    let data_dir = data_dir.canonicalize()?;
    let lock = crate::migration_io::open_lock_file(&data_dir.join("node.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock).context("runtime data directory is in use")?;
    let binding_path = data_dir.join("host-binding.json");
    crate::migration_io::reject_symlink(&binding_path, "host binding")?;
    let binding: HostBinding = serde_json::from_slice(&std::fs::read(binding_path)?)?;
    ensure!(
        binding.database.is_absolute() && binding.database.is_file(),
        "existing absolute host database required"
    );
    crate::migration_io::reject_symlink(&binding.database, "host database")?;
    let host = FleetStore::open(&binding.database).await?;
    let runtime = host
        .runtimes()
        .await?
        .into_iter()
        .find(|r| r.id == binding.runtime_id)
        .context("runtime is not registered with this host")?;
    let configured_dir = Path::new(
        runtime.config["data_dir"]
            .as_str()
            .context("runtime data directory missing")?,
    )
    .canonicalize()?;
    ensure!(
        configured_dir == data_dir,
        "runtime data directory binding mismatch"
    );
    let unit = runtime.config["unit"]
        .as_str()
        .context("runtime unit missing")?;
    let process_proof = proof::stopped(unit).await?;
    let owner = host
        .owner(run_id)
        .await?
        .context("Brain runtime ownership missing")?;
    ensure!(
        owner.runtime_id == binding.runtime_id,
        "Brain belongs to another runtime"
    );
    let layout = DirectoryLayout::new(data_dir.clone(), workflow_root.map(Path::to_path_buf))?;
    let mut journal = Journal::load(layout.clone())?;
    let record = journal
        .records
        .get(run_id)
        .context("Brain journal missing")?;
    ensure!(
        record.assignment.index.kind == ExecutionKind::Brain,
        "not a Brain root"
    );
    let ticket = record
        .queue
        .as_ref()
        .and_then(|queue| queue.ticket.clone())
        .context("Brain capacity ticket missing")?;
    let (ticket_execution, ticket_runtime, ticket_phase) = host
        .capacity_ticket(&ticket)
        .await?
        .context("capacity ticket missing")?;
    ensure!(
        ticket_execution == run_id && ticket_runtime == binding.runtime_id,
        "capacity ticket ownership mismatch"
    );
    let database = data_dir.join("runtime.db");
    ensure!(database.is_file(), "runtime database missing");
    for path in [
        &database,
        &data_dir.join("runtime.db-wal"),
        &data_dir.join("runtime.db-shm"),
    ] {
        crate::migration_io::reject_symlink(path, "runtime database")?;
    }
    let store = LibsqlStore::open(database).await?;
    let snapshot = store.brain_layered(run_id).await?;
    let intent = Intent {
        data_dir: data_dir.clone(),
        host_database: binding.database.canonicalize()?,
        runtime_id: binding.runtime_id.clone(),
        run_id: run_id.into(),
        ticket: ticket.clone(),
        generation: snapshot.as_ref().map(|s| s.run.generation),
        record_sha256: crate::migration_io::sha256_file(
            &layout.record_path(ExecutionKind::Brain, run_id)?,
        )?,
        proof: process_proof,
    };
    crate::migration_io::reject_symlink(receipt_dir, "receipt directory")?;
    opencoder_core::share_fs::durable_create_dir_all(receipt_dir)?;
    let intent_path = receipt_dir.join(format!("{run_id}.intent.json"));
    crate::migration_io::reject_symlink(&intent_path, "settlement intent")?;
    if intent_path.exists() {
        let previous: Intent = serde_json::from_slice(&std::fs::read(&intent_path)?)?;
        ensure!(
            previous.data_dir == intent.data_dir
                && previous.host_database == intent.host_database
                && previous.runtime_id == intent.runtime_id
                && previous.run_id == intent.run_id
                && previous.ticket == intent.ticket,
            "settlement receipt ownership mismatch"
        );
        if snapshot.as_ref().is_none_or(|s| !s.run.phase.terminal()) {
            ensure!(
                previous.generation == intent.generation
                    && previous.record_sha256 == intent.record_sha256,
                "Brain changed since settlement intent; refusing stale receipt"
            );
        }
        ensure!(
            snapshot
                .as_ref()
                .is_none_or(|s| !s.run.phase.terminal() || s.run.phase == LayeredPhase::Failed),
            "Brain already ended with another result"
        );
    } else {
        ensure!(
            ticket_phase != "done",
            "already completed capacity has no settlement intent"
        );
        ensure!(
            snapshot
                .as_ref()
                .is_none_or(|s| !s.run.phase.terminal() || s.run.phase == LayeredPhase::Failed),
            "Brain already ended with another result"
        );
        // The receipt exists before container cleanup or the first state write.
        crate::migration_io::durable_json(&intent_path, &intent)?;
    }
    #[cfg(not(windows))]
    opencoder_dag_runtime::sandbox::runc::cleanup_owned_run(
        &layout
            .checked_kind_root(ExecutionKind::Brain)?
            .join("bundles"),
        run_id,
    )
    .await?;
    proof::stopped(unit).await?;
    let snapshot = fail_root(&store, &mut journal, run_id, REASON).await?;
    ensure!(
        snapshot.run.phase == LayeredPhase::Failed,
        "Brain did not settle as failed"
    );
    host.finish_capacity(&ticket, &binding.runtime_id).await?;
    let result = json!({"run_id":run_id,"runtime_id":binding.runtime_id,"phase":"failed", "ticket":ticket,"capacity":"done","resumed":false});
    let result_path = receipt_dir.join(format!("{run_id}.result.json"));
    crate::migration_io::reject_symlink(&result_path, "settlement result")?;
    crate::migration_io::durable_json(&result_path, &result)?;
    Ok(result)
}
