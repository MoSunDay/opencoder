//! Freeze the complete run input before starting any container or step.
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::{io::Write, path::Path};

fn read(path: &Path) -> Result<Value> {
    let bytes = crate::sandbox::output_limit::read_file_bounded(
        path,
        "DAG inputs",
        crate::sandbox::output_limit::STRUCTURED_JSON_LIMIT_BYTES,
    )?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub(super) fn load(root: &Path, resume: bool) -> Result<Value> {
    let accepted = root.join("input.json");
    let input = match std::fs::symlink_metadata(&accepted) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "DAG input must be a regular file"
            );
            read(&accepted)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Null,
        Err(error) => return Err(error.into()),
    };
    let path = root.join("inputs.json");
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) => ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "frozen DAG inputs must be a regular file"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            ensure!(
                !resume,
                "frozen DAG inputs missing; recovery requires the original snapshot"
            );
            let temporary = root.join(format!(".inputs-{}.tmp", ulid::Ulid::new()));
            let published = (|| -> Result<()> {
                let mut file = opencoder_core::platform::fs::create_private_file(&temporary)?;
                file.write_all(&serde_json::to_vec(&input)?)?;
                file.sync_all()?;
                match opencoder_core::platform::fs::publish_new(&temporary, &path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
                    Err(error) => Err(error.into()),
                }
            })();
            let cleanup = std::fs::remove_file(&temporary);
            published?;
            cleanup.context("remove DAG input staging file")?;
            opencoder_core::platform::fs::sync_directory(root)?;
        }
        Err(error) => return Err(error.into()),
    }
    let frozen = read(&path)?;
    ensure!(
        frozen == input,
        "frozen DAG inputs changed; restore the accepted input before recovery"
    );
    Ok(frozen)
}
