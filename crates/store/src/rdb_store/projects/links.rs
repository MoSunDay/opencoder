//! TODO↔execution assignments (`pex/<todo>/<exec>`): a reference, never a
//! copy of the execution's state. `link` is INSERT-OR-IGNORE by
//! `(todo_id, execution_id)`; ordering mirrors the SQL `ORDER BY
//! created_at DESC, execution_id DESC`.

use anyhow::{Context, Result};

use crate::project::ProjectAssignment;
use crate::rdb_store::projects::keys::{execution_key, execution_prefix, execution_root};
use crate::rdb_store::tx;

fn row(store: &rdb::store::Store, key: &[u8]) -> Result<ProjectAssignment> {
    let value = rdb::store::get(store, b"", key)
        .map_err(crate::rdb_store::kv::engine_err)?
        .context("execution link row is missing")?;
    crate::rdb_store::kv::decode::<ProjectAssignment>(&value)
}

/// One todo's assignments, newest first.
pub fn list(store: &rdb::store::Store, todo_id: &str) -> Result<Vec<ProjectAssignment>> {
    let mut rows = Vec::new();
    for key in rdb::store::prefix_keys_collect(store, &execution_prefix(todo_id), 0) {
        rows.push(row(store, &key)?);
    }
    rows.sort_by(|a, b| (b.created_at, &b.execution_id).cmp(&(a.created_at, &a.execution_id)));
    Ok(rows)
}

/// The newest assignment of every todo.
pub fn latest(store: &rdb::store::Store) -> Result<Vec<ProjectAssignment>> {
    let mut all: Vec<ProjectAssignment> = Vec::new();
    for key in rdb::store::prefix_keys_collect(store, &execution_root(), 0) {
        all.push(row(store, &key)?);
    }
    all.sort_by(|a, b| {
        (&a.todo_id, b.created_at, &b.execution_id).cmp(&(
            &b.todo_id,
            a.created_at,
            &a.execution_id,
        ))
    });
    let mut out = Vec::new();
    for assignment in all {
        if out
            .last()
            .is_none_or(|last: &ProjectAssignment| last.todo_id != assignment.todo_id)
        {
            out.push(assignment);
        }
    }
    Ok(out)
}

/// INSERT-OR-IGNORE by `(todo_id, execution_id)`: re-linking an existing
/// pair is a no-op that keeps the original `created_at`.
pub fn link(store: &rdb::store::Store, assignment: &ProjectAssignment) -> Result<()> {
    let key = execution_key(&assignment.todo_id, &assignment.execution_id);
    if rdb::store::get(store, b"", &key)
        .map_err(crate::rdb_store::kv::engine_err)?
        .is_some()
    {
        return Ok(());
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, assignment)?;
    tx::commit(store, batch)
}

/// `false` when the pair does not exist.
pub fn unlink(store: &rdb::store::Store, todo_id: &str, execution_id: &str) -> Result<bool> {
    let key = execution_key(todo_id, execution_id);
    if rdb::store::get(store, b"", &key)
        .map_err(crate::rdb_store::kv::engine_err)?
        .is_none()
    {
        return Ok(false);
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::del(&mut batch, &key);
    tx::commit(store, batch)?;
    Ok(true)
}
