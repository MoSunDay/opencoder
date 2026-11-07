//! Project-goal rows (`pgl/<id>`): CRUD with patches from `Some` fields,
//! detach-on-delete and per-scope summary history.

use anyhow::{Context, Result};

use crate::project::summary::SummaryScope;
use crate::project_types::{ProjectGoalPatch, ProjectGoalRecord};
use crate::rdb_store::kv::get_json;
use crate::rdb_store::projects::keys::{goal_key, tag_key, tag_scope_index};
use crate::rdb_store::projects::{people, summary, tags};
use crate::rdb_store::{index, tx};

pub fn get(store: &rdb::store::Store, id: &str) -> Result<Option<ProjectGoalRecord>> {
    get_json(store, &goal_key(id))
}

/// Ordered by `sort` then `created_at` (id as deterministic tiebreak).
pub fn list(store: &rdb::store::Store) -> Result<Vec<ProjectGoalRecord>> {
    let mut rows = Vec::new();
    for (key, value) in tx::scan_prefix(store, b"pgl/")? {
        rows.push(
            crate::rdb_store::kv::decode::<ProjectGoalRecord>(&value)
                .with_context(|| format!("decode {}", String::from_utf8_lossy(&key)))?,
        );
    }
    rows.sort_by(|a, b| (a.sort, a.created_at, &a.id).cmp(&(b.sort, b.created_at, &b.id)));
    Ok(rows)
}

pub fn create(store: &rdb::store::Store, rec: &ProjectGoalRecord) -> Result<()> {
    anyhow::ensure!(
        get(store, &rec.id)?.is_none(),
        "insert project goal: id {} already exists",
        rec.id
    );
    people::validate(store, &rec.people.replacement())?;
    let mut rec = rec.clone();
    rec.people = people::normalize(&rec.people);
    let mut batch = rocksdb::WriteBatch::default();
    summary::append(
        store,
        &mut batch,
        SummaryScope::Project,
        &rec.id,
        &rec.summary.current_summary,
        rec.summary.summary_updated_at.unwrap_or(rec.created_at),
        None,
    )?;
    tx::put(&mut batch, &goal_key(&rec.id), &rec)?;
    tx::commit(store, batch)
}

/// `false` = id not found; always stamps `updated_at = now_ms`.
pub fn patch(
    store: &rdb::store::Store,
    id: &str,
    patch: &ProjectGoalPatch,
    now_ms: i64,
) -> Result<bool> {
    let Some(mut rec) = get(store, id)? else {
        return Ok(false);
    };
    people::validate(store, &patch.people)?;
    if let Some(v) = &patch.title {
        rec.title = v.clone();
    }
    if let Some(v) = &patch.detail_md {
        rec.detail_md = Some(v.clone());
    }
    if let Some(v) = patch.status {
        rec.status = v;
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
            SummaryScope::Project,
            id,
            text,
            now_ms,
            None,
        )?;
    }
    tx::put(&mut batch, &goal_key(id), &rec)?;
    tx::commit(store, batch)?;
    Ok(true)
}

/// Delete the goal; its initiatives detach to standalone (goal_id NULL),
/// their TODOs and runs survive. Project-scoped tags drop with their owner.
pub fn delete(store: &rdb::store::Store, id: &str) -> Result<bool> {
    if get(store, id)?.is_none() {
        return Ok(false);
    }
    let previous_tags = tags::list(store)?;
    let mut initiatives = super::initiatives::list_all(store)?;
    let mut batch = rocksdb::WriteBatch::default();
    for initiative in initiatives
        .iter_mut()
        .filter(|i| i.goal_id.as_deref() == Some(id))
    {
        initiative.goal_id = None;
        tx::put(
            &mut batch,
            &super::keys::initiative_key(&initiative.id),
            initiative,
        )?;
    }
    tx::del(&mut batch, &goal_key(id));
    summary::remove(store, &mut batch, SummaryScope::Project, id)?;
    let mut effective = previous_tags.clone();
    for tag in previous_tags
        .iter()
        .filter(|t| t.scope_type == "project" && t.scope_id == id)
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
        initiatives,
        todos: super::todos::list_all(store)?,
    };
    tags::stage_reconcile(store, &mut batch, &snap, None)?;
    tx::commit(store, batch)?;
    Ok(true)
}
