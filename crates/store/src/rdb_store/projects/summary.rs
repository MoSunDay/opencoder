//! Bounded summary history (`project_summary_versions`): one row per
//! `(scope, id, version)`, trimmed to the last
//! [`SUMMARY_HISTORY_LIMIT`] versions. Writes share the caller's batch.

use anyhow::Result;
use serde_json::json;

use crate::project::summary::{SummaryScope, SummaryVersion, SUMMARY_HISTORY_LIMIT};
use crate::rdb_store::kv::{get_json, pad_ord};
use crate::rdb_store::projects::keys::{goal_key, initiative_key, summary_prefix};
use crate::rdb_store::{seq, tx};

/// Owner row key per scope (existence + restore target).
fn owner_key(scope: SummaryScope, id: &str) -> Vec<u8> {
    match scope {
        SummaryScope::Project => goal_key(id),
        SummaryScope::Initiative => initiative_key(id),
    }
}

pub fn owner_exists(store: &rdb::store::Store, scope: SummaryScope, id: &str) -> Result<bool> {
    Ok(rdb::store::get(store, b"", &owner_key(scope, id))
        .map_err(crate::rdb_store::kv::engine_err)?
        .is_some())
}

/// Append the next version (text trimmed) and trim the history tail in the
/// same batch.
pub fn append(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    scope: SummaryScope,
    id: &str,
    text: &str,
    now: i64,
    restored_from: Option<i64>,
) -> Result<SummaryVersion> {
    let prefix = summary_prefix(scope, id);
    let mut version = 0;
    for (key, _value) in tx::scan_prefix(store, &prefix)? {
        if let Some(v) = seq::parse_ord(&key, &prefix) {
            version = version.max(v);
        }
    }
    version += 1;
    let record = SummaryVersion {
        version,
        current_summary: text.trim().into(),
        created_at: now,
        restored_from,
    };
    let mut key = prefix.clone();
    key.extend_from_slice(pad_ord(version).as_bytes());
    tx::put(batch, &key, &record)?;
    for stale in 1..=(version - SUMMARY_HISTORY_LIMIT) {
        let mut key = prefix.clone();
        key.extend_from_slice(pad_ord(stale).as_bytes());
        tx::del(batch, &key);
    }
    Ok(record)
}

pub fn remove(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    scope: SummaryScope,
    id: &str,
) -> Result<()> {
    tx::delete_prefix(store, batch, &summary_prefix(scope, id))
}

/// Newest first, history limit applied; `None` when the owner row is gone.
pub fn list(
    store: &rdb::store::Store,
    scope: SummaryScope,
    id: &str,
) -> Result<Option<Vec<SummaryVersion>>> {
    if !owner_exists(store, scope, id)? {
        return Ok(None);
    }
    let prefix = summary_prefix(scope, id);
    let mut rows = Vec::new();
    for (_key, value) in tx::scan_prefix(store, &prefix)? {
        rows.push(crate::rdb_store::kv::decode::<SummaryVersion>(&value)?);
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.version));
    rows.truncate(SUMMARY_HISTORY_LIMIT as usize);
    Ok(Some(rows))
}

/// Restore an old version onto its owner and append it as the newest
/// version; `None` when the version or the owner row is missing. The owner
/// row is patched through its JSON view so goals and initiatives share the
/// code path (`current_summary`, `summary_updated_at`, `updated_at`).
pub fn restore(
    store: &rdb::store::Store,
    scope: SummaryScope,
    id: &str,
    version: i64,
    now: i64,
) -> Result<Option<SummaryVersion>> {
    let prefix = summary_prefix(scope, id);
    let mut wanted = prefix.clone();
    wanted.extend_from_slice(pad_ord(version).as_bytes());
    let Some(found) = tx::scan_prefix(store, &prefix)?
        .into_iter()
        .find(|(key, _)| *key == wanted)
    else {
        return Ok(None);
    };
    let text = crate::rdb_store::kv::decode::<SummaryVersion>(&found.1)?.current_summary;
    let key = owner_key(scope, id);
    let Some(mut row) = get_json::<serde_json::Value>(store, &key)? else {
        return Ok(None);
    };
    let obj = row
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("project owner row is not an object"))?;
    obj.insert("current_summary".into(), json!(text));
    obj.insert("summary_updated_at".into(), json!(now));
    obj.insert("updated_at".into(), json!(now));
    let mut batch = rocksdb::WriteBatch::default();
    tx::put_raw(&mut batch, &key, &crate::rdb_store::kv::encode(&row)?);
    let record = append(store, &mut batch, scope, id, &text, now, Some(version))?;
    tx::commit(store, batch)?;
    Ok(Some(record))
}
