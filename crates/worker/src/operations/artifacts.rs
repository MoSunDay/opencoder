use crate::Worker;
use anyhow::{bail, Result};
use base64::Engine;
use opencoder_core::fleet::*;
use serde_json::Value;
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
        id,
        legacy,
        input["step"].as_str().unwrap_or(""),
        input["file"].as_str().unwrap_or("output.txt"),
        input["offset"].as_u64().unwrap_or(0),
        None,
    )
    .await
}

pub(super) async fn read_request(worker: &Worker, request: ArtifactRequest) -> Result<RpcReply> {
    if let Some(reply) = super::validate_reference(worker, &request.execution).await? {
        return Ok(reply);
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
    read_fields(
        worker,
        &request.execution.id,
        legacy,
        &request.step,
        &request.file,
        request.offset,
        request.version.as_deref(),
    )
    .await
}

async fn read_fields(
    worker: &Worker,
    id: &str,
    legacy: bool,
    step: &str,
    name: &str,
    offset: u64,
    expected_version: Option<&str>,
) -> Result<RpcReply> {
    let workflow_root = if legacy {
        worker.inner.layout.checked_legacy_workflow_root()?
    } else {
        worker.inner.layout.kind_root(ExecutionKind::Dag)
    };
    if !matches!(name, "output.txt" | "output.json" | "meta.json") {
        return Ok(RpcReply::error(400, "unknown artifact file"));
    }
    let path = opencoder_dag::artifacts::step_dir(&workflow_root, id, step)
        .map_err(anyhow::Error::msg)?
        .join(name);
    if !path.exists() {
        return Ok(RpcReply::error(404, "artifact not available"));
    }
    let root = workflow_root.join(id).canonicalize()?;
    let path = path.canonicalize()?;
    if !path.starts_with(root) {
        bail!("artifact path escaped execution directory");
    }
    let mut file = tokio::fs::File::open(path).await?;
    let metadata = file.metadata().await?;
    let total = metadata.len();
    let version = file_version(&metadata);
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
    file.take(ARTIFACT_CHUNK_BYTES as u64)
        .read_to_end(&mut bytes)
        .await?;
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
