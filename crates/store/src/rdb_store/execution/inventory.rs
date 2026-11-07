//! Execution-inventory sources: commit-ordered change cursor over session
//! and project-run rows, never message or artifact bodies.
//!
//! Key family `ivc/<seq:020>/<source>/<id>` replays the SQL
//! `inventory_source_changes` log (seq is the max committed change seq + 1;
//! the `/source/id` suffix lets one atomic batch stage several changes at
//! the same seq without overwriting each other). Writers keep the log
//! pruned to the last 100000 entries with a range tombstone, the KV form
//! of the trigger-side `DELETE ... WHERE seq < MAX(seq)-100000`.

use anyhow::Result;

use crate::inventory::{InventorySources, ProjectIndexSource, SessionIndexSource};
use crate::rdb_store::kv::{self, pad_ord, ORD_WIDTH};

/// Retention window (SQL trigger prune bound).
const KEEP: i64 = 100_000;

pub const SOURCE_SESSION: &str = "session";
pub const SOURCE_PROJECT: &str = "project";

fn change_prefix() -> Vec<u8> {
    b"ivc/".to_vec()
}

fn change_key(seq: i64, source: &str, id: &str) -> Vec<u8> {
    format!("ivc/{}/{}/{}", pad_ord(seq), source, id).into_bytes()
}

/// Highest committed change seq (0 when the log is empty).
fn last_seq(store: &rdb::store::Store) -> i64 {
    let mut from = change_prefix();
    from.extend_from_slice(pad_ord(i64::MAX).as_bytes());
    let mut max = 0i64;
    let _ = rdb::store::for_each_down_from(store, &from, false, &mut |key, _| {
        if let Some(seq) = parse_seq(key) {
            max = seq;
        }
        false
    });
    max
}

/// Lowest committed change seq (0 when the log is empty).
fn first_seq(store: &rdb::store::Store) -> i64 {
    rdb::store::first_key_from(store, &change_prefix())
        .ok()
        .flatten()
        .and_then(|key| parse_seq(&key))
        .unwrap_or(0)
}

fn parse_seq(key: &[u8]) -> Option<i64> {
    let suffix = key.strip_prefix(b"ivc/")?;
    let text = std::str::from_utf8(suffix.get(..ORD_WIDTH)?).ok()?;
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// One inventory change staged inside the caller's atomic batch. The seq is
/// derived from the committed log head, so two staging helpers contributing
/// to the same batch allocate the same seq under different ids — exactly
/// one SQL trigger batch — and a failed batch moves nothing.
pub fn stage(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    changes: &[(&str, &str)],
) -> Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let seq = last_seq(store) + 1;
    for (source, id) in changes {
        batch.put(change_key(seq, source, id), []);
    }
    if seq > KEEP {
        // Prune inside the same atomic batch (the trigger-side retention
        // DELETE); a failed batch moves neither the change nor the prune.
        let upper = format!("ivc/{}", pad_ord(seq - KEEP)).into_bytes();
        batch.delete_range(change_prefix(), upper);
    }
    Ok(())
}

/// Is this session row in the execution inventory lane? Top-level parents
/// without a subagent child, plus Agent-step detail rows; operator lanes
/// stay invisible to node inventories.
fn in_lane(store: &rdb::store::Store, meta: &crate::types::SessionMeta) -> bool {
    if meta.kind.as_deref() == Some("operator") {
        return false;
    }
    let task_type = meta
        .task_type
        .as_deref()
        .unwrap_or(crate::types::TASK_TYPE_PARENT);
    task_type == crate::types::TASK_TYPE_AGENT_STEP
        || (task_type == crate::types::TASK_TYPE_PARENT
            && !crate::rdb_store::index::any_under(
                store,
                &crate::rdb_store::kv::subagent_child_prefix(&meta.id),
            ))
}

/// Every `project_todo_runs` row, decoded straight from its KV row.
fn project_runs(store: &rdb::store::Store) -> Result<Vec<crate::ProjectTodoRunRecord>> {
    let mut runs = Vec::new();
    for (_, value) in crate::rdb_store::tx::scan_prefix(store, b"prn/")? {
        runs.push(kv::decode(&value)?);
    }
    Ok(runs)
}

/// Distinct `(source, id)` pairs changed after `from`, in log order.
fn changed_after(store: &rdb::store::Store, from: u64) -> Result<Vec<(String, String, u64)>> {
    let mut start = change_prefix();
    // SQL semantics are `seq > from`; `for_each_from(.., excl=true)` only
    // skips an exact match, so start strictly after the cursor revision.
    start.extend_from_slice(pad_ord(from as i64 + 1).as_bytes());
    let mut seen = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    rdb::store::for_each_from(store, &start, true, &mut |key, _| {
        let Some(suffix) = key.strip_prefix(b"ivc/") else {
            return false;
        };
        let Some((seq, rest)) = split_seq(suffix) else {
            return true;
        };
        if let Some((source, id)) = split_source_id(rest) {
            if seen.insert((source.to_owned(), id.to_owned())) {
                result.push((source.to_owned(), id.to_owned(), seq));
            }
        }
        true
    })
    .map_err(kv::engine_err)?;
    Ok(result)
}

fn split_seq(suffix: &[u8]) -> Option<(u64, &[u8])> {
    let text = std::str::from_utf8(suffix.get(..ORD_WIDTH)?).ok()?;
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((text.parse().ok()?, suffix.get(ORD_WIDTH + 1..)?))
}

fn split_source_id(rest: &[u8]) -> Option<(&str, &str)> {
    let text = std::str::from_utf8(rest).ok()?;
    let (source, id) = text.split_once('/')?;
    Some((source, id))
}

/// Incremental source rows with a transactionally maintained change
/// cursor: `full` when the caller's cursor expired, otherwise only the
/// changed ids plus their current rows (removals first, re-adds after).
pub fn sources(store: &rdb::store::Store, after: Option<u64>) -> Result<InventorySources> {
    let first = first_seq(store) as u64;
    let revision = last_seq(store) as u64;
    let full = after.is_none_or(|v| v > revision || v.saturating_add(1) < first);
    let mut result = InventorySources {
        revision,
        full,
        ..Default::default()
    };
    if !full && after == Some(revision) {
        return Ok(result);
    }
    let changed = if full {
        Vec::new()
    } else {
        changed_after(store, after.unwrap_or(0))?
    };
    for row in crate::rdb_store::conversation::sessions::all_rows(store)? {
        let meta = &row.meta;
        if !full
            && !changed
                .iter()
                .any(|(s, id, _)| s == SOURCE_SESSION && id == &meta.id)
        {
            continue;
        }
        if in_lane(store, meta) {
            result.sessions.push(SessionIndexSource {
                id: meta.id.clone(),
                kind: meta.kind.clone(),
                title: meta.title.clone(),
                created_at: meta.created_at,
            });
        }
    }
    for run in project_runs(store)? {
        if !full
            && !changed
                .iter()
                .any(|(s, id, _)| s == SOURCE_PROJECT && id == &run.id)
        {
            continue;
        }
        result.project_runs.push(ProjectIndexSource {
            id: run.id.clone(),
            todo_id: run.todo_id.clone(),
            created_at: run.created_at,
            status: run.status,
        });
    }
    if !full {
        for (source, id, _) in changed {
            if source == SOURCE_SESSION {
                result.removed_sessions.push(id);
            } else {
                result.removed_runs.push(id);
            }
        }
    }
    Ok(result)
}
