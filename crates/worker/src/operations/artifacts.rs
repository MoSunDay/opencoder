use crate::Worker;
use anyhow::{bail, Result};
use base64::Engine;
use opencoder_core::fleet::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
#[cfg(test)]
mod tests;
pub(super) async fn read(worker: &Worker, id: &str, input: Value) -> Result<RpcReply> {
    let journal = worker.inner.journal.lock().await;
    if !journal.records.get(id).is_some_and(|record| {
        record.assignment.request.kind == ExecutionKind::Dag
            && record.assignment.index.kind == ExecutionKind::Dag
    }) {
        return Ok(RpcReply::error(400, "artifacts require a DAG execution"));
    }
    drop(journal);
    read_fields(
        worker,
        &ArtifactRequest {
            execution: ExecutionRef {
                id: id.into(),
                kind: ExecutionKind::Dag,
            },
            step: input["step"].as_str().unwrap_or("").into(),
            index: input["index"].as_u64().map(|i| i as usize),
            file: input["file"].as_str().unwrap_or("output.txt").into(),
            offset: input["offset"].as_u64().unwrap_or(0),
            version: None,
        },
    )
    .await
}

pub(super) async fn read_request(worker: &Worker, request: ArtifactRequest) -> Result<RpcReply> {
    if let Some(reply) = super::validate_reference(worker, &request.execution).await? {
        return Ok(reply);
    }
    if request.step == "brain-result" && request.file.starts_with("artifacts-") {
        let name = request
            .file
            .strip_prefix("artifacts-")
            .and_then(|v| v.strip_suffix(".json"));
        let Some(digest) =
            name.filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
        else {
            return Ok(RpcReply::error(400, "invalid artifact index name"));
        };
        if request.index.is_some() {
            return Ok(RpcReply::error(400, "artifact index has no instance index"));
        }
        let managed = worker
            .inner
            .journal
            .lock()
            .await
            .records
            .get(&request.execution.id)
            .is_some_and(|record| {
                crate::brain::v4::parent::leaf(&record.assignment.request.input)
                    || crate::brain::v4::parent::root(&record.assignment.request.input)
            });
        if !managed {
            return Ok(RpcReply::error(404, "managed artifact index not found"));
        }
        let dir = worker
            .inner
            .layout
            .execution_dir(request.execution.kind, &request.execution.id)?
            .join("brain-result");
        let path = dir.join(&request.file);
        crate::brain::workspace::checked(&dir, &path)?;
        if !path.try_exists()? {
            return Ok(RpcReply::error(404, "artifact index not available"));
        }
        let expected = (std::fs::metadata(&path)?.len(), digest.to_ascii_lowercase());
        return read_chunk(
            &path,
            &request.step,
            &request.file,
            request.offset,
            request.version.as_deref(),
            Some(&expected),
        )
        .await;
    }
    if request.step == "brain-result" && request.file == "output.json" {
        let managed = worker
            .inner
            .journal
            .lock()
            .await
            .records
            .get(&request.execution.id)
            .is_some_and(|r| r.assignment.request.input.get("_brain").is_some());
        if !managed {
            return Ok(RpcReply::error(404, "managed result not found"));
        }
        let path = worker
            .inner
            .layout
            .execution_dir(request.execution.kind, &request.execution.id)?
            .join("brain-result/output.json");
        return read_chunk(
            &path,
            &request.step,
            &request.file,
            request.offset,
            request.version.as_deref(),
            None,
        )
        .await;
    }
    if request.execution.kind == ExecutionKind::Project {
        return super::query::project::artifact(worker, request).await;
    }
    if request.step == "workspace" && request.execution.kind != ExecutionKind::Dag {
        if request.index.is_some() {
            return Ok(RpcReply::error(
                400,
                "workspace artifacts have no instance index",
            ));
        }
        let dir = {
            let journal = worker.inner.journal.lock().await;
            let record = journal
                .records
                .get(&request.execution.id)
                .ok_or_else(|| anyhow::anyhow!("execution missing"))?;
            if crate::brain::v4::parent::leaf(&record.assignment.request.input)
                || crate::brain::v4::parent::root(&record.assignment.request.input)
            {
                Some(crate::brain::workdir::for_record(worker, record)?)
            } else {
                crate::brain::workspace::workdir(record)?
            }
        };
        let Some(dir) = dir else {
            return Ok(RpcReply::error(404, "execution has no shared workspace"));
        };
        return read_directory(&dir, &request, true).await;
    }
    if request.execution.kind != ExecutionKind::Dag {
        return Ok(RpcReply::error(400, "artifacts require a DAG execution"));
    }
    read_fields(worker, &request).await
}

async fn read_fields(worker: &Worker, request: &ArtifactRequest) -> Result<RpcReply> {
    let id = &request.execution.id;
    let step = &request.step;
    let index = request.index;
    let workflow_root = {
        let journal = worker.inner.journal.lock().await;
        crate::layout::dag::accepted_parent(
            journal
                .records
                .get(id)
                .ok_or_else(|| anyhow::anyhow!("DAG execution missing"))?,
        )?
    };
    let dir = opencoder_dag::artifacts::execution_dir(&workflow_root, id, step, index)
        .map_err(anyhow::Error::msg)?;
    if !dir.exists() {
        return Ok(RpcReply::error(404, "artifact not available"));
    }
    let mut confined = workflow_root.clone();
    for part in dir.strip_prefix(&workflow_root)?.components() {
        confined.push(part);
        anyhow::ensure!(
            !std::fs::symlink_metadata(&confined)?
                .file_type()
                .is_symlink(),
            "artifact directory cannot be a symlink"
        );
    }
    read_directory(&dir, request, false).await
}

async fn read_directory(
    dir: &std::path::Path,
    request: &ArtifactRequest,
    shared: bool,
) -> Result<RpcReply> {
    let name = request.file.as_str();
    if name.is_empty()
        || name.contains('\\')
        || name
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || std::path::Path::new(name).is_absolute()
    {
        return Ok(RpcReply::error(400, "invalid artifact path"));
    }
    let declared = if matches!(
        name,
        "output.txt" | "output.json" | "meta.json" | "artifacts.json"
    ) || (shared && matches!(name, "context.json" | "plan.json" | "inputs.json"))
    {
        None
    } else {
        let Some(value) = declaration(dir, name).await? else {
            return Ok(RpcReply::error(400, "unknown artifact file"));
        };
        Some(value)
    };
    let path = dir.join(name);
    crate::brain::workspace::checked(dir, &path)?;
    if !path.exists() {
        return Ok(RpcReply::error(404, "artifact not available"));
    }
    let root = dir.canonicalize()?;
    let path = path.canonicalize()?;
    if !path.starts_with(root) {
        bail!("artifact path escaped execution directory");
    }
    read_chunk(
        &path,
        &request.step,
        name,
        request.offset,
        request.version.as_deref(),
        declared.as_ref(),
    )
    .await
}

async fn declaration(dir: &std::path::Path, name: &str) -> Result<Option<(u64, String)>> {
    for file in ["artifacts.json", "output.json"] {
        if dir.join(file).is_symlink() {
            bail!("artifact declaration cannot be a symlink");
        }
        let bytes = match tokio::fs::read(dir.join(file)).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let value: Value = serde_json::from_slice(&bytes)?;
        let row = if file == "artifacts.json" {
            value["files"].as_array().and_then(|rows| {
                rows.iter().find(|row| {
                    row.get("path")
                        .or_else(|| row.get("file"))
                        .and_then(Value::as_str)
                        == Some(name)
                })
            })
        } else {
            value.get("report_archive").filter(|row| {
                row.get("path")
                    .or_else(|| row.get("file"))
                    .and_then(Value::as_str)
                    == Some(name)
            })
        };
        if let Some(row) = row {
            let (Some(bytes), Some(hash)) = (row["bytes"].as_u64(), row["sha256"].as_str()) else {
                bail!("artifact declaration lacks bytes or SHA-256");
            };
            anyhow::ensure!(
                hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()),
                "invalid artifact SHA-256"
            );
            return Ok(Some((bytes, hash.to_ascii_lowercase())));
        }
    }
    Ok(None)
}

async fn read_chunk(
    path: &std::path::Path,
    step: &str,
    name: &str,
    offset: u64,
    expected_version: Option<&str>,
    declared: Option<&(u64, String)>,
) -> Result<RpcReply> {
    if !path.is_file() {
        return Ok(RpcReply::error(404, "artifact not available"));
    }
    let mut file = tokio::fs::File::open(path).await?;
    let metadata = file.metadata().await?;
    let total = metadata.len();
    let version = file_version(&metadata);
    if let Some((bytes, hash)) = declared {
        if *bytes != total {
            return Ok(RpcReply::error(409, "artifact differs from declared size"));
        }
        if offset == 0 || expected_version.is_none() {
            let mut digest = Sha256::new();
            let mut buffer = vec![0; 64 * 1024];
            loop {
                let count = file.read(&mut buffer).await?;
                if count == 0 {
                    break;
                }
                digest.update(&buffer[..count]);
            }
            if format!("{:x}", digest.finalize()) != *hash {
                return Ok(RpcReply::error(
                    409,
                    "artifact differs from declared SHA-256",
                ));
            }
        }
    }
    if expected_version.is_some_and(|expected| expected != version) {
        return Ok(RpcReply::error(
            409,
            "artifact changed while it was being streamed",
        ));
    }
    if offset > total {
        return Ok(RpcReply::error(416, "artifact offset beyond end"));
    }
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(ARTIFACT_CHUNK_BYTES as u64)
        .read_to_end(&mut bytes)
        .await?;
    if file_version(&file.metadata().await?) != version {
        return Ok(RpcReply::error(409, "artifact changed while reading"));
    }
    let next = offset + bytes.len() as u64;
    Ok(RpcReply::ok(serde_json::to_value(ArtifactChunk {
        file: name.to_owned(),
        step: step.to_owned(),
        offset,
        next_offset: next,
        total_bytes: total,
        version,
        eof: next >= total,
        encoding: "base64".into(),
        bytes_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
    })?))
}

#[cfg(unix)]
fn file_version(metadata: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!(
        "{}-{}-{}-{}-{}",
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec()
    )
}

#[cfg(not(unix))]
fn file_version(metadata: &std::fs::Metadata) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{modified}", metadata.len())
}
