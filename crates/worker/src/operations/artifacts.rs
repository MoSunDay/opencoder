use crate::Worker;
use anyhow::{bail, Result};
use base64::Engine;
use opencoder_core::fleet::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
pub(super) async fn read(worker: &Worker, id: &str, input: Value) -> Result<RpcReply> {
    let journal = worker.inner.journal.lock().await;
    if !journal.records.get(id).is_some_and(|record| {
        record.assignment.request.kind == ExecutionKind::Dag
            && record.assignment.index.kind == ExecutionKind::Dag
    }) {
        return Ok(RpcReply::error(400, "artifacts require a DAG execution"));
    }
    let legacy = journal.uses_legacy(id);
    drop(journal);
    read_fields(
        worker,
        legacy,
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
    if request.execution.kind != ExecutionKind::Dag {
        return Ok(RpcReply::error(400, "artifacts require a DAG execution"));
    }
    let legacy = worker
        .inner
        .journal
        .lock()
        .await
        .uses_legacy(&request.execution.id);
    read_fields(worker, legacy, &request).await
}

async fn read_fields(worker: &Worker, legacy: bool, request: &ArtifactRequest) -> Result<RpcReply> {
    let id = &request.execution.id;
    let step = &request.step;
    let index = request.index;
    let name = request.file.as_str();
    let offset = request.offset;
    let expected_version = request.version.as_deref();
    let workflow_root = if legacy {
        worker.inner.layout.checked_legacy_workflow_root()?
    } else {
        worker.inner.layout.kind_root(ExecutionKind::Dag)
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
    ) {
        None
    } else {
        let Some(value) = declaration(&dir, name).await? else {
            return Ok(RpcReply::error(400, "unknown artifact file"));
        };
        Some(value)
    };
    let path = dir.join(name);
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
        step,
        name,
        offset,
        expected_version,
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
            value["files"]
                .as_array()
                .and_then(|rows| rows.iter().find(|row| row["path"] == name))
        } else {
            value
                .get("report_archive")
                .filter(|row| row["file"] == name)
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
