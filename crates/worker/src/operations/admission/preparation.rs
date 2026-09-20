//! Reserve an immutable request before cold I/O, independently of other IDs.
use crate::Worker;
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::{Assignment, ExecutionKind, RpcReply};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};

const FILE: &str = "pending-create.json";

/// Hand off the async worker while preserving task-local resource roots and
/// thread-local configuration isolation on the thread doing synchronous I/O.
/// Single-threaded callers are used by in-memory unit tests only.
pub(in crate::operations) fn blocking<T>(work: impl FnOnce() -> T) -> T {
    if tokio::runtime::Handle::try_current()
        .is_ok_and(|runtime| runtime.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
    {
        tokio::task::block_in_place(work)
    } else {
        work()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingCreate {
    schema_version: u8,
    assignment: Assignment,
}

/// Caller holds this execution's lifecycle lock and the short admission gate.
/// Publishing the directory and reservation together keeps arbitrary orphan
/// directories rejected, while an interrupted preparation can replay its input.
pub(in crate::operations) fn begin(
    worker: &Worker,
    mut assignment: Assignment,
) -> Result<Result<Assignment, RpcReply>> {
    let root = worker
        .inner
        .layout
        .execution_dir(assignment.index.kind, &assignment.index.id)?;
    match fs::symlink_metadata(&root) {
        Ok(meta) => {
            ensure!(
                meta.is_dir() && !meta.file_type().is_symlink(),
                "execution preparation root must be a real directory"
            );
            let file = root.join(FILE);
            let metadata = match fs::symlink_metadata(&file) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Err(RpcReply::error(
                        409,
                        "execution directory exists without a durable journal record",
                    )));
                }
                Err(error) => return Err(error.into()),
            };
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "execution preparation must be a regular file"
            );
            let saved: PendingCreate = serde_json::from_slice(&fs::read(file)?)
                .context("invalid pending execution preparation")?;
            ensure!(
                saved.schema_version == 1,
                "unsupported execution preparation version"
            );
            ensure!(
                saved.assignment.index.id == assignment.index.id
                    && saved.assignment.index.kind == assignment.index.kind
                    && saved.assignment.index.node_id == assignment.index.node_id,
                "execution preparation ownership mismatch"
            );
            if assignment.request.kind == ExecutionKind::Project
                && assignment.request.input.get("run_id").is_none()
            {
                assignment.request.input["run_id"] =
                    saved.assignment.request.input["run_id"].clone();
            }
            if saved.assignment.request != assignment.request {
                return Ok(Err(RpcReply::error(
                    409,
                    "execution id already preparing with different input",
                )));
            }
            return Ok(Ok(saved.assignment));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if assignment.request.kind == ExecutionKind::Project {
        super::super::project_admission::ensure_id(&mut assignment.request.input)?;
    }
    let parent = root
        .parent()
        .context("execution preparation parent missing")?;
    opencoder_core::share_fs::durable_create_dir_all(parent)?;
    let stage = parent.join(format!(".prepare-{}", ulid::Ulid::new()));
    fs::create_dir(&stage)?;
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(stage.join(FILE))?;
        file.write_all(&serde_json::to_vec(&PendingCreate {
            schema_version: 1,
            assignment: assignment.clone(),
        })?)?;
        file.sync_all()?;
        fs::File::open(&stage)?.sync_all()?;
        fs::rename(&stage, &root)?;
        fs::File::open(parent)?.sync_all()?;
        Ok::<_, anyhow::Error>(())
    })();
    if result.is_err() && stage.exists() {
        // Only our unpublished reservation is removed; execution data and
        // existing journals are never deleted by admission recovery.
        let _ = fs::remove_file(stage.join(FILE));
        let _ = fs::remove_dir(&stage);
    }
    result?;
    Ok(Ok(assignment))
}

pub(in crate::operations) fn finish(root: &Path) -> Result<()> {
    match fs::remove_file(root.join(FILE)) {
        Ok(()) => fs::File::open(root)?.sync_all().map_err(Into::into),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
