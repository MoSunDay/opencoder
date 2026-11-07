//! Initiative rows (`pin/<id>`): CRUD with the nonempty-deletion guard,
//! atomic order replacement and per-scope summary history.

use anyhow::{Context, Result};

use crate::project::summary::SummaryScope;
use crate::project::InitiativeOrderConflict;
use crate::project_types::{ProjectInitiativePatch, ProjectInitiativeRecord};
use crate::rdb_store::kv::get_json;
use crate::rdb_store::projects::keys::{initiative_key, tag_key, tag_scope_index};
use crate::rdb_store::projects::{people, summary, tags};
use crate::rdb_store::{index, tx};

pub fn get(store: &rdb::store::Store, id: &str) -> Result<Option<ProjectInitiativeRecord>> {
    get_json(store, &initiative_key(id))
}

/// Ordered by `sort`, `created_at`, id.
pub fn list_all(store: &rdb::store::Store) -> Result<Vec<ProjectInitiativeRecord>> {
    let mut rows = Vec::new();
    for (key, value) in tx::scan_prefix(store, b"pin/")? {
        rows.push(
            crate::rdb_store::kv::decode::<ProjectInitiativeRecord>(&value)
                .with_context(|| format!("decode {}", String::from_utf8_lossy(&key)))?,
        );
    }
    rows.sort_by(|a, b| (a.sort, a.created_at, &a.id).cmp(&(b.sort, b.created_at, &b.id)));
    Ok(rows)
}

/// `goal_id == None` lists across all goals.
pub fn list(
    store: &rdb::store::Store,
    goal_id: Option<&str>,
) -> Result<Vec<ProjectInitiativeRecord>> {
    Ok(list_all(store)?
        .into_iter()
        .filter(|i| goal_id.is_none_or(|g| i.goal_id.as_deref() == Some(g)))
        .collect())
}

pub fn create(store: &rdb::store::Store, rec: &ProjectInitiativeRecord) -> Result<()> {
    anyhow::ensure!(
        get(store, &rec.id)?.is_none(),
        "insert project initiative: id {} already exists",
        rec.id
    );
    people::validate(store, &rec.people.replacement())?;
    let mut rec = rec.clone();
    rec.people = people::normalize(&rec.people);
    let mut batch = rocksdb::WriteBatch::default();
    summary::append(
        store,
        &mut batch,
        SummaryScope::Initiative,
        &rec.id,
        &rec.summary.current_summary,
        rec.summary.summary_updated_at.unwrap_or(rec.created_at),
        None,
    )?;
    tx::put(&mut batch, &initiative_key(&rec.id), &rec)?;
    tx::commit(store, batch)
}

/// `false` = id not found; always stamps `updated_at = now_ms`. A moved
/// initiative re-resolves tag selections for its todos in the same batch.
pub fn patch(
    store: &rdb::store::Store,
    id: &str,
    patch: &ProjectInitiativePatch,
    now_ms: i64,
) -> Result<bool> {
    let Some(mut rec) = get(store, id)? else {
        return Ok(false);
    };
    people::validate(store, &patch.people)?;
    if let Some(v) = &patch.goal_id {
        rec.goal_id = v.clone();
    }
    if let Some(v) = &patch.title {
        rec.title = v.clone();
    }
    if let Some(v) = &patch.detail_md {
        rec.detail_md = Some(v.clone());
    }
    if let Some(v) = patch.sort {
        rec.sort = v;
    }
    if let Some(text) = &patch.current_summary {
        rec.summary.current_summary = text.trim().into();
        rec.summary.summary_updated_at = Some(now_ms);
    }
    people::apply(&mut rec.people, &patch.people);
    rec.updated_at = now_ms;
    let mut batch = rocksdb::WriteBatch::default();
    if let Some(text) = &patch.current_summary {
        summary::append(
            store,
            &mut batch,
            SummaryScope::Initiative,
            id,
            text,
            now_ms,
            None,
        )?;
    }
    tx::put(&mut batch, &initiative_key(id), &rec)?;
    if patch.goal_id.is_some() {
        let mut snap = tags::Snapshot::read(store)?;
        if let Some(slot) = snap.initiatives.iter_mut().find(|i| i.id == id) {
            *slot = rec.clone();
        }
        tags::stage_reconcile(store, &mut batch, &snap, None)?;
    }
    tx::commit(store, batch)?;
    Ok(true)
}

/// Only empty initiatives can be deleted; associations must be moved or
/// unlinked explicitly (`InitiativeNotEmpty`).
pub fn delete(store: &rdb::store::Store, id: &str) -> Result<bool> {
    if get(store, id)?.is_none() {
        return Ok(false);
    }
    anyhow::ensure!(
        !super::todos::list_all(store)?
            .iter()
            .any(|t| t.initiative_id.as_deref() == Some(id)),
        crate::project::InitiativeNotEmpty
    );
    let previous_tags = tags::list(store)?;
    let mut batch = rocksdb::WriteBatch::default();
    tx::del(&mut batch, &initiative_key(id));
    summary::remove(store, &mut batch, SummaryScope::Initiative, id)?;
    let mut effective = previous_tags.clone();
    for tag in previous_tags
        .iter()
        .filter(|t| t.scope_type == "initiative" && t.scope_id == id)
    {
        tx::del(&mut batch, &tag_key(&tag.id));
        index::remove(
            &mut batch,
            &tag_scope_index(&tag.scope_type, &tag.scope_id, &tag.name),
        );
        effective.retain(|t| t.id != tag.id);
    }
    let snap = tags::Snapshot {
        previous_tags,
        tags: effective,
        initiatives: list_all(store)?,
        todos: super::todos::list_all(store)?,
    };
    tags::stage_reconcile(store, &mut batch, &snap, None)?;
    tx::commit(store, batch)?;
    Ok(true)
}

/// Replace a goal's complete initiative order, only while the observed
/// order still matches. Conflicts ([`InitiativeOrderConflict`]) leave every
/// row untouched; `false` means the goal is missing.
pub fn reorder(
    store: &rdb::store::Store,
    goal_id: &str,
    expected_ids: &[String],
    ids: &[String],
    now_ms: i64,
) -> Result<bool> {
    if super::goals::get(store, goal_id)?.is_none() {
        return Ok(false);
    }
    let current: Vec<String> = list(store, Some(goal_id))?
        .into_iter()
        .map(|i| i.id)
        .collect();
    let submitted: std::collections::HashSet<&String> = ids.iter().collect();
    if current != expected_ids
        || ids.len() != current.len()
        || submitted.len() != ids.len()
        || current.iter().any(|id| !submitted.contains(id))
    {
        return Err(InitiativeOrderConflict.into());
    }
    let mut batch = rocksdb::WriteBatch::default();
    for (index, id) in ids.iter().enumerate() {
        let Some(mut rec) = get(store, id)? else {
            return Err(InitiativeOrderConflict.into());
        };
        rec.sort = index as i64;
        rec.updated_at = now_ms;
        tx::put(&mut batch, &initiative_key(id), &rec)?;
    }
    tx::commit(store, batch)?;
    Ok(true)
}
