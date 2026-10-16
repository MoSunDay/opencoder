//! Node-owned, verified file references. Never enumerate an entire workspace.
use crate::{journal::Record, Worker};
use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::ArtifactRef, fleet::ExecutionRef};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

pub(crate) fn json(path: &Path) -> Result<Value> {
    super::super::workspace::checked(path.parent().context("file parent missing")?, path)?;
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "artifact metadata must be a regular file"
    );
    ensure!(
        file.metadata()?.len() <= opencoder_core::fleet::MAX_FRAME_BYTES as u64,
        "artifact metadata exceeds transport capacity; split the declared manifest"
    );
    Ok(serde_json::from_reader(file)?)
}

pub(crate) fn reference(
    execution: &ExecutionRef,
    dir: &Path,
    step: &str,
    index: Option<usize>,
    file: &str,
) -> Result<ArtifactRef> {
    let path = dir.join(file);
    super::super::workspace::checked(dir, &path)?;
    let mut input = File::open(&path)?;
    let before = input.metadata()?;
    ensure!(before.is_file(), "artifact must be a regular file");
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .context("artifact size overflow")?;
        digest.update(&buffer[..count]);
    }
    let after = std::fs::metadata(&path)?;
    ensure!(
        bytes == before.len()
            && after.len() == before.len()
            && after.modified()? == before.modified()?,
        "artifact changed while indexing"
    );
    Ok(ArtifactRef {
        execution: execution.clone(),
        step: step.into(),
        index,
        file: file.into(),
        sha256: format!("{:x}", digest.finalize()),
        bytes,
    })
}

pub(crate) fn directory(
    execution: &ExecutionRef,
    dir: &Path,
    step: &str,
    index: Option<usize>,
    metadata: bool,
) -> Result<Vec<ArtifactRef>> {
    let mut documents = Vec::new();
    for file in ["artifacts.json", "output.json"] {
        let path = dir.join(file);
        if path.try_exists()? || path.is_symlink() {
            documents.push((file, json(&path)?));
        }
    }
    let mut refs = Vec::new();
    for (file, bytes, digest) in opencoder_dag::artifacts::parse_declarations(&documents)? {
        let actual = reference(execution, dir, step, index, &file)?;
        ensure!(
            actual.bytes == bytes && actual.sha256 == digest,
            "artifact differs from declared size or SHA-256: {file}"
        );
        refs.push(actual);
    }
    if metadata {
        for file in ["output.json", "output.txt", "meta.json"] {
            refs.push(reference(execution, dir, step, index, file)?);
        }
    }
    Ok(refs)
}

pub(crate) fn workspace(worker: &Worker, record: &Record) -> Result<Vec<ArtifactRef>> {
    directory(
        &record.assignment.index.execution_ref(),
        &super::super::workdir::for_record(worker, record)?,
        "workspace",
        None,
        false,
    )
}
