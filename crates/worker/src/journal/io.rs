use super::Record;
use anyhow::{bail, Context, Result};
use opencoder_core::fleet::{valid_id, ExecutionKind};
use serde_json::Value;
use std::{io::Write, path::Path};

pub(crate) fn read_record(path: &Path, allow_legacy_kind: bool) -> Result<Record> {
    let mut value: Value = serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("read {}", path.display()))?,
    )
    .with_context(|| format!("parse {}", path.display()))?;
    if allow_legacy_kind {
        normalize_legacy_index(&mut value)?;
    }
    let record: Record = serde_json::from_value(value)?;
    validate_record(&record, None)?;
    Ok(record)
}

pub(crate) fn same_execution(left: &Record, right: &Record) -> bool {
    left.assignment.request == right.assignment.request
        && left.assignment.definition == right.assignment.definition
        && left.assignment.index.id == right.assignment.index.id
        && left.assignment.index.kind == right.assignment.index.kind
        && left.assignment.index.node_id == right.assignment.index.node_id
        && left.assignment.index.created_at == right.assignment.index.created_at
}

pub(super) fn validate_record(record: &Record, expected_kind: Option<ExecutionKind>) -> Result<()> {
    if !valid_id(&record.assignment.index.id) {
        bail!("invalid journal ID");
    }
    if record.assignment.index.id != record.assignment.request.id
        || record.assignment.index.kind != record.assignment.request.kind
    {
        bail!("journal assignment identity or kind mismatch");
    }
    if expected_kind.is_some_and(|kind| kind != record.assignment.index.kind) {
        bail!("journal path kind does not match execution kind");
    }
    Ok(())
}

pub(super) fn normalize_legacy_index(value: &mut Value) -> Result<()> {
    let request_kind = value
        .pointer("/assignment/request/kind")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("journal request kind missing"))?;
    let index = value
        .pointer_mut("/assignment/index")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow::anyhow!("journal execution index missing"))?;
    if !index.contains_key("kind") {
        index.insert("kind".into(), request_kind);
    }
    Ok(())
}

pub(super) fn durable_json(path: &Path, value: &Record) -> Result<()> {
    let parent = path.parent().context("journal path has no parent")?;
    opencoder_core::share_fs::durable_create_dir_all(parent)?;
    let temp = parent.join(format!(".execution.tmp-{}", ulid::Ulid::new()));
    let mut file = std::fs::File::create(&temp)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    std::fs::rename(&temp, path)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
