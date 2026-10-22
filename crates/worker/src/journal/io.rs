use super::Record;
use anyhow::{bail, Context, Result};
use opencoder_core::fleet::{valid_id, ExecutionKind};
use serde_json::Value;
use std::{io::Write, path::Path};

pub(super) fn read_stamped(path: &Path, legacy: bool) -> Result<(Record, super::cache::Stamp)> {
    let before = super::cache::Stamp::of(&std::fs::metadata(path)?);
    let record = read_record(path, legacy)?;
    let after = super::cache::Stamp::of(&std::fs::metadata(path)?);
    anyhow::ensure!(before == after, "execution record changed during load");
    Ok((record, after))
}

pub(crate) fn read_record(path: &Path, allow_legacy_kind: bool) -> Result<Record> {
    parse_record(
        &std::fs::read(path).with_context(|| format!("read {}", path.display()))?,
        allow_legacy_kind,
    )
    .map_err(|error| anyhow::anyhow!("parse {}: {error:#}", path.display()))
}

pub(super) fn parse_record(bytes: &[u8], allow_legacy_kind: bool) -> Result<Record> {
    let record: Record = if allow_legacy_kind {
        let mut value: Value = serde_json::from_slice(bytes)?;
        normalize_legacy_index(&mut value)?;
        serde_json::from_value(value)?
    } else {
        serde_json::from_slice(bytes)?
    };
    validate_record(&record, None)?;
    Ok(record)
}

pub(crate) fn same_execution(left: &Record, right: &Record) -> bool {
    left.assignment.request == right.assignment.request
        && left.assignment.private_context == right.assignment.private_context
        && left.assignment.codex == right.assignment.codex
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

pub(super) fn durable_json(path: &Path, value: &Record) -> Result<Option<super::cache::Stamp>> {
    let parent = path.parent().context("journal path has no parent")?;
    opencoder_core::share_fs::durable_create_dir_all(parent)?;
    let temp = parent.join(format!(".execution.tmp-{}", ulid::Ulid::new()));
    let file = opencoder_core::platform::fs::create_private_file(&temp)?;
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer(&mut writer, value)?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    let written = writer.get_ref().metadata().ok();
    opencoder_core::platform::fs::replace(&temp, path)?;
    opencoder_core::platform::fs::sync_directory(parent)?;
    // Read metadata from our still-open file, not the path: a concurrent
    // replacement must never associate our view with another file's identity.
    // Cache metadata failure must not turn a durable save into a failed save.
    Ok(written
        .zip(writer.get_ref().metadata().ok())
        .and_then(|(before, after)| super::cache::Stamp::published(&before, &after)))
}
