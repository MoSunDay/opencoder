//! Transactional tag catalog and TODO selection changes.
//!
//! The SQL backend re-reads tags/initiatives/todos inside its transaction
//! (read-your-writes); a RocksDB batch is invisible until commit, so
//! callers pass a [`Snapshot`] describing the effective post-tx state and
//! reconciliation diffs it against the links still on disk. Every helper
//! stages into the caller's batch, keeping catalog + links atomic.

use anyhow::Result;

use crate::project::tags::{reconcile_links, validate_selection, TagError};
use crate::project::{ProjectTag, ProjectTodoTag};
use crate::project_types::{ProjectInitiativeRecord, ProjectTodoPatch, ProjectTodoRecord};
use crate::rdb_store::kv::{self, get_json};
use crate::rdb_store::projects::keys::{
    tag_key, tag_root, tag_scope_index, todo_tag_key, todo_tag_root,
};
use crate::rdb_store::{index, tx};

/// Effective catalog + container state for one reconciling transaction.
pub struct Snapshot {
    pub previous_tags: Vec<ProjectTag>,
    pub tags: Vec<ProjectTag>,
    pub initiatives: Vec<ProjectInitiativeRecord>,
    pub todos: Vec<ProjectTodoRecord>,
}

impl Snapshot {
    /// Read the committed state (previous == current).
    pub fn read(store: &rdb::store::Store) -> Result<Self> {
        let tags = list(store)?;
        Ok(Snapshot {
            previous_tags: tags.clone(),
            tags,
            initiatives: super::initiatives::list_all(store)?,
            todos: super::todos::list_all(store)?,
        })
    }
}

/// Catalog ordered by `(name, id)` — the SQL `ORDER BY name,id`.
pub fn list(store: &rdb::store::Store) -> Result<Vec<ProjectTag>> {
    let mut rows = Vec::new();
    for (_key, value) in tx::scan_prefix(store, &tag_root())? {
        rows.push(kv::decode::<ProjectTag>(&value)?);
    }
    rows.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
    Ok(rows)
}

/// Selections ordered by `(todo_id, tag_id)` — the SQL `ORDER BY todo_id,tag_id`.
pub fn links(store: &rdb::store::Store) -> Result<Vec<ProjectTodoTag>> {
    let mut rows = Vec::new();
    for (_key, value) in tx::scan_prefix(store, &todo_tag_root())? {
        rows.push(kv::decode::<ProjectTodoTag>(&value)?);
    }
    rows.sort_by(|a, b| (&a.todo_id, &a.tag_id).cmp(&(&b.todo_id, &b.tag_id)));
    Ok(rows)
}

/// Stage the link diff that reconciles on-disk links with the snapshot.
pub fn stage_reconcile(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    snap: &Snapshot,
    selection: Option<(&str, &[String])>,
) -> Result<()> {
    let mut old_links = links(store)?;
    if let Some((todo_id, ids)) = selection {
        let todo = snap
            .todos
            .iter()
            .find(|t| t.id == todo_id)
            .ok_or(TagError::InvalidScope)?;
        let initiative = snap
            .initiatives
            .iter()
            .find(|i| todo.initiative_id.as_ref() == Some(&i.id));
        validate_selection(&snap.tags, initiative, ids)?;
        old_links.retain(|link| link.todo_id != todo_id);
        old_links.extend(ids.iter().map(|id| ProjectTodoTag {
            todo_id: todo_id.into(),
            tag_id: id.clone(),
        }));
    }
    let desired = reconcile_links(
        &snap.previous_tags,
        &snap.tags,
        &snap.initiatives,
        &snap.todos,
        &old_links,
    );
    let current = links(store)?;
    for link in current.iter().filter(|link| !desired.contains(link)) {
        tx::del(batch, &todo_tag_key(&link.todo_id, &link.tag_id));
    }
    for link in desired.iter().filter(|link| !current.contains(link)) {
        tx::put(batch, &todo_tag_key(&link.todo_id, &link.tag_id), link)?;
    }
    Ok(())
}

/// `write_tag`: scope/owner validation, duplicate-name rejection, upsert by
/// id (moving the unique index entry), then link reconciliation.
pub fn write(store: &rdb::store::Store, tag: &ProjectTag) -> Result<()> {
    let previous = list(store)?;
    match tag.scope_type.as_str() {
        "global" if tag.scope_id.is_empty() => {}
        "project" => anyhow::ensure!(
            super::goals::get(store, &tag.scope_id)?.is_some(),
            TagError::InvalidScope
        ),
        "initiative" => anyhow::ensure!(
            super::initiatives::get(store, &tag.scope_id)?.is_some(),
            TagError::InvalidScope
        ),
        _ => anyhow::bail!(TagError::InvalidScope),
    }
    anyhow::ensure!(
        !previous.iter().any(|t| t.id != tag.id
            && t.scope_type == tag.scope_type
            && t.scope_id == tag.scope_id
            && t.name == tag.name),
        TagError::Duplicate
    );
    let mut batch = rocksdb::WriteBatch::default();
    let key = tag_key(&tag.id);
    if let Some(old) = previous.iter().find(|t| t.id == tag.id) {
        index::remove(
            &mut batch,
            &tag_scope_index(&old.scope_type, &old.scope_id, &old.name),
        );
    }
    index::claim_unique(
        store,
        &mut batch,
        &tag_scope_index(&tag.scope_type, &tag.scope_id, &tag.name),
        &key,
        "project_tags.scope_type, project_tags.scope_id, project_tags.name",
    )?;
    tx::put(&mut batch, &key, tag)?;
    let mut effective = previous.clone();
    if let Some(slot) = effective.iter_mut().find(|t| t.id == tag.id) {
        *slot = tag.clone();
    } else {
        effective.push(tag.clone());
    }
    let snap = Snapshot {
        initiatives: super::initiatives::list_all(store)?,
        todos: super::todos::list_all(store)?,
        previous_tags: previous,
        tags: effective,
    };
    stage_reconcile(store, &mut batch, &snap, None)?;
    tx::commit(store, batch)
}

/// `delete_tag`: drop the row + index entry, then reconcile. `false` when
/// the id is unknown.
pub fn delete(store: &rdb::store::Store, id: &str) -> Result<bool> {
    let previous = list(store)?;
    let Some(existing) = previous.iter().find(|t| t.id == id).cloned() else {
        return Ok(false);
    };
    let mut batch = rocksdb::WriteBatch::default();
    tx::del(&mut batch, &tag_key(id));
    index::remove(
        &mut batch,
        &tag_scope_index(&existing.scope_type, &existing.scope_id, &existing.name),
    );
    let effective: Vec<ProjectTag> = previous.iter().filter(|t| t.id != id).cloned().collect();
    let snap = Snapshot {
        initiatives: super::initiatives::list_all(store)?,
        todos: super::todos::list_all(store)?,
        previous_tags: previous,
        tags: effective,
    };
    stage_reconcile(store, &mut batch, &snap, None)?;
    tx::commit(store, batch)?;
    Ok(true)
}

/// `create_todo_tagged`: insert the todo, then validate/apply its selection.
pub fn create_todo(
    store: &rdb::store::Store,
    todo: &ProjectTodoRecord,
    ids: &[String],
) -> Result<()> {
    anyhow::ensure!(
        get_json::<ProjectTodoRecord>(store, &super::keys::todo_key(&todo.id))?.is_none(),
        "insert project todo: id {} already exists",
        todo.id
    );
    super::people::validate(store, &todo.people.replacement())?;
    let mut todo = todo.clone();
    todo.people = super::people::normalize(&todo.people);
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &super::keys::todo_key(&todo.id), &todo)?;
    let mut snap = Snapshot::read(store)?;
    snap.todos.push(todo.clone());
    stage_reconcile(store, &mut batch, &snap, Some((&todo.id, ids)))?;
    tx::commit(store, batch)
}

/// `patch_todo_tagged` / `patch_todo_accepted`: optional acceptance
/// precondition, patch, then selection reconciliation when the todo moved
/// between containers or the selection was replaced.
pub fn patch_todo(
    store: &rdb::store::Store,
    id: &str,
    patch: &ProjectTodoPatch,
    ids: Option<&[String]>,
    expected: Option<&crate::project::acceptance::TodoAcceptance>,
    now: i64,
) -> Result<bool> {
    if let Some(expected) = expected {
        anyhow::ensure!(
            expected.permits_patch(patch),
            "acceptance cannot change task requirements"
        );
        super::acceptance::check(store, id, expected)?;
    }
    let Some(mut rec) = get_json::<ProjectTodoRecord>(store, &super::keys::todo_key(id))? else {
        return Ok(false);
    };
    super::people::validate(store, &patch.people)?;
    super::todos::apply_patch(&mut rec, patch);
    rec.updated_at = now;
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &super::keys::todo_key(id), &rec)?;
    if patch.initiative_id.is_some() || ids.is_some() {
        let mut snap = Snapshot::read(store)?;
        if let Some(slot) = snap.todos.iter_mut().find(|t| t.id == id) {
            *slot = rec.clone();
        }
        stage_reconcile(store, &mut batch, &snap, ids.map(|ids| (id, ids)))?;
    }
    tx::commit(store, batch)?;
    Ok(true)
}
