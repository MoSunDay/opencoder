//! Read-only candidates for catalog removal. Completed execution history is retained.
use super::keys;
use super::FleetStore;
use crate::rdb_store::tx;
use anyhow::{ensure, Result};
use serde_json::Value;

const RETAINED_STATUS: [&str; 3] = ["done", "error", "cancelled"];
const CLOSED_PHASE: [&str; 6] = [
    "accepted",
    "rejected",
    "done",
    "complete",
    "completed",
    "cancelled",
];

pub(super) fn catalog_reference_candidates(
    store: &rdb::store::Store,
    name: &str,
) -> Result<Vec<(String, Value)>> {
    ensure!(!name.is_empty(), "catalog reference name is empty");
    // Escaped/non-ASCII identifiers can have several JSON spellings. In
    // that case inspect every candidate instead of risking a false negative.
    let needle = if name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        name
    } else {
        ""
    };
    let mut result = Vec::new();
    for (key, value) in tx::scan_prefix(store, b"def/")? {
        let Some((kind, id)) = split_def_key(&key) else {
            continue;
        };
        if contains(&value, needle) {
            result.push((
                format!("definition:{kind}:{id}"),
                serde_json::from_slice(&value)?,
            ));
        }
    }
    for (key, value) in tx::scan_prefix(store, b"bpv/")? {
        let Some(label) = plan_label(&key) else {
            continue;
        };
        if contains(&value, needle) {
            result.push((label, serde_json::from_slice(&value)?));
        }
    }
    for (key, value) in tx::scan_prefix(store, b"asg/")? {
        let Some(id) = strip_prefix_str(&key, b"asg/") else {
            continue;
        };
        if execution_retained(store, id) || !contains(&value, needle) {
            continue;
        }
        result.push((format!("execution:{id}"), serde_json::from_slice(&value)?));
    }
    for (key, value) in tx::scan_prefix(store, b"rcp/")? {
        let Some((scope, id)) = split_def_key(&key) else {
            continue;
        };
        let receipt: super::handoff::Receipt = crate::rdb_store::kv::decode(&value)?;
        if CLOSED_PHASE.contains(&receipt.phase.as_str()) || !contains(&value, needle) {
            continue;
        }
        result.push((format!("dispatch:{scope}:{id}"), receipt.payload));
    }
    Ok(result)
}

/// Completed executions no longer reference live catalog entries.
fn execution_retained(store: &rdb::store::Store, id: &str) -> bool {
    matches!(
        keys::read_index(store, id).ok().flatten(),
        Some(record) if RETAINED_STATUS.contains(&record.status.as_str())
    )
}

fn contains(raw: &[u8], needle: &str) -> bool {
    std::str::from_utf8(raw)
        .map(|text| text.contains(needle))
        .unwrap_or(false)
}

/// Split `def/<kind>/<id>` / `rcp/<scope>/<id>` keys into their two parts.
fn split_def_key(key: &[u8]) -> Option<(&str, &str)> {
    let text = std::str::from_utf8(key).ok()?;
    let (_, suffix) = text.split_once('/')?;
    let (kind, id) = suffix.split_once('/')?;
    Some((kind, id))
}

fn strip_prefix_str<'a>(key: &'a [u8], prefix: &[u8]) -> Option<&'a str> {
    std::str::from_utf8(key.strip_prefix(prefix)?).ok()
}

fn plan_label(key: &[u8]) -> Option<String> {
    let suffix = strip_prefix_str(key, b"bpv/")?;
    let (id, version) = suffix.split_once('/')?;
    let version = crate::rdb_store::seq::parse_ord(version.as_bytes(), b"")?;
    Some(format!("plan:{id}@{version}"))
}

#[path = "access/catalog.rs"]
mod database_access;
