//! Project TODO rows (`ptd/<id>`): plain CRUD, the running-claim CAS pair,
//! cascade deletion and atomic board reorder. Tag-aware create/patch and
//! acceptance-checked patch live in [`super::tags`].

use anyhow::{Context, Result};

use crate::project_types::{
    ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunRecord,
    ProjectTodoRunStatus, ProjectTodoStatus,
};
use crate::rdb_store::kv::get_json;
use crate::rdb_store::projects::keys::{
    comment_prefix, comment_request_prefix, execution_prefix, run_index_prefix, todo_key,
    todo_prefix, todo_tag_prefix,
};
use crate::rdb_store::projects::{runs, runs_inspect};
use crate::rdb_store::tx;
use crate::ProjectTodoSummary;

pub fn get(store: &rdb::store::Store, id: &str) -> Result<Option<ProjectTodoRecord>> {
    get_json(store, &todo_key(id))
}

/// Ordered by `board_status, position, created_at, id` (the SQL ORDER BY).
pub fn list_all(store: &rdb::store::Store) -> Result<Vec<ProjectTodoRecord>> {
    let mut rows = Vec::new();
    for (key, value) in tx::scan_prefix(store, &todo_prefix())? {
        rows.push(
            crate::rdb_store::kv::decode::<ProjectTodoRecord>(&value)
                .with_context(|| format!("decode {}", String::from_utf8_lossy(&key)))?,
        );
    }
    rows.sort_by(|a, b| {
        (&a.board_status, a.position, a.created_at, &a.id).cmp(&(
            &b.board_status,
            b.position,
            b.created_at,
            &b.id,
        ))
    });
    Ok(rows)
}

/// `initiative_id == None` lists ALL todos (backlog included).
pub fn list(
    store: &rdb::store::Store,
    initiative_id: Option<&str>,
) -> Result<Vec<ProjectTodoRecord>> {
    Ok(list_all(store)?
        .into_iter()
        .filter(|t| initiative_id.is_none_or(|m| t.initiative_id.as_deref() == Some(m)))
        .collect())
}

/// Pure projection of a patch's `Some` fields; `Some(None)` clears an
/// optional field, `None` leaves it untouched.
pub fn apply_patch(rec: &mut ProjectTodoRecord, patch: &ProjectTodoPatch) {
    if let Some(v) = &patch.title {
        rec.title = v.clone();
    }
    if let Some(v) = &patch.draft {
        rec.draft = v.clone();
    }
    if let Some(v) = &patch.plan_md {
        rec.plan_md = v.clone();
    }
    if let Some(v) = patch.status {
        rec.status = v;
    }
    if let Some(v) = &patch.board_status {
        rec.board_status = v.clone();
    }
    if let Some(v) = patch.position {
        rec.position = v;
    }
    if let Some(v) = &patch.capability_id {
        rec.capability_id = v.clone();
    }
    if let Some(v) = &patch.agent {
        rec.agent = v.clone();
    }
    if let Some(v) = patch.executor_kind {
        rec.executor_kind = v;
    }
    if let Some(v) = &patch.executor_ref {
        rec.executor_ref = v.clone();
    }
    if let Some(v) = &patch.executor_spec {
        rec.executor_spec = v.clone();
    }
    if let Some(v) = &patch.initiative_id {
        rec.initiative_id = v.clone();
    }
    if let Some(v) = &patch.active_session_id {
        rec.active_session_id = v.clone();
    }
    super::people::apply(&mut rec.people, &patch.people);
}

/// Flip a not-yet-running todo into running; `false` = missing or already
/// running (both mean "no claim").
pub fn claim_running(store: &rdb::store::Store, id: &str, now_ms: i64) -> Result<bool> {
    let Some(mut rec) = get(store, id)? else {
        return Ok(false);
    };
    if rec.status == ProjectTodoStatus::Running {
        return Ok(false);
    }
    rec.status = ProjectTodoStatus::Running;
    rec.updated_at = now_ms;
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &todo_key(id), &rec)?;
    tx::commit(store, batch)?;
    Ok(true)
}

/// One atomic batch owns the conditional claim AND the run insert, so a
/// concurrent caller cannot observe a claimed todo without its run — and a
/// duplicate run id aborts both (nothing is committed).
pub fn claim_running_with_run(
    store: &rdb::store::Store,
    rec: &ProjectTodoRunRecord,
    now_ms: i64,
) -> Result<bool> {
    anyhow::ensure!(
        rec.status == ProjectTodoRunStatus::Running,
        "atomic todo claim requires a running run"
    );
    let Some(mut todo) = get(store, &rec.todo_id)? else {
        return Ok(false);
    };
    if todo.status == ProjectTodoStatus::Running {
        return Ok(false);
    }
    if runs_inspect::has_running(store, &rec.todo_id)? {
        return Ok(false);
    }
    let mut accepted = rec.clone();
    accepted.version = runs_inspect::next_version(store, &rec.todo_id);
    let mut batch = rocksdb::WriteBatch::default();
    if rec.kind == ProjectTodoRunKind::Execute {
        todo.status = ProjectTodoStatus::Running;
        todo.updated_at = now_ms;
        tx::put(&mut batch, &todo_key(&todo.id), &todo)?;
    }
    runs::stage(store, &mut batch, &accepted)?;
    tx::commit(store, batch)?;
    Ok(true)
}

/// Expected-status CAS variant: applies only while the row still holds
/// `when`; `false` = not found or the state moved on.
pub fn patch_when(
    store: &rdb::store::Store,
    id: &str,
    when: ProjectTodoStatus,
    patch: &ProjectTodoPatch,
    now_ms: i64,
) -> Result<bool> {
    let Some(mut rec) = get(store, id)? else {
        return Ok(false);
    };
    if rec.status != when {
        return Ok(false);
    }
    super::people::validate(store, &patch.people)?;
    apply_patch(&mut rec, patch);
    rec.updated_at = now_ms;
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &todo_key(id), &rec)?;
    tx::commit(store, batch)?;
    Ok(true)
}

/// Tx cascade: executions, runs (+ membership index), tag links and
/// comments go with the todo. `false` when the id does not exist.
pub fn delete(store: &rdb::store::Store, id: &str) -> Result<bool> {
    if get(store, id)?.is_none() {
        return Ok(false);
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::del(&mut batch, &todo_key(id));
    let mut removed_runs: Vec<String> = Vec::new();
    for (key, value) in tx::scan_prefix(store, &run_index_prefix(id))? {
        tx::del(&mut batch, &key);
        let run_id =
            String::from_utf8(value).map_err(|_| anyhow::anyhow!("invalid run index owner"))?;
        tx::del(
            &mut batch,
            &crate::rdb_store::projects::keys::run_key(&run_id),
        );
        removed_runs.push(run_id);
    }
    // Inventory change log (SQL trigger on project_todo_runs DELETE): one
    // row per removed run, staged in the same atomic batch.
    let changes: Vec<(&str, &str)> = removed_runs
        .iter()
        .map(|run_id| ("project", run_id.as_str()))
        .collect();
    crate::rdb_store::execution::inventory::stage(store, &mut batch, &changes)?;
    tx::delete_prefix(store, &mut batch, &execution_prefix(id))?;
    tx::delete_prefix(store, &mut batch, &todo_tag_prefix(id))?;
    tx::delete_prefix(store, &mut batch, &comment_prefix(id))?;
    tx::delete_prefix(store, &mut batch, &comment_request_prefix(id))?;
    tx::commit(store, batch)?;
    Ok(true)
}

/// Bounded projection (`ProjectTodoSummary`): oversized text fields are
/// reported as byte totals instead of payloads.
pub fn get_summary(store: &rdb::store::Store, id: &str) -> Result<Option<ProjectTodoSummary>> {
    let Some(rec) = get(store, id)? else {
        return Ok(None);
    };
    let draft = runs_inspect::bounded_text(&rec.draft, format!("project.todo.{id}.draft"));
    Ok(Some(ProjectTodoSummary {
        id: rec.id,
        initiative_id: rec.initiative_id,
        title: rec.title,
        draft,
        plan_md: rec
            .plan_md
            .as_deref()
            .map(|v| runs_inspect::bounded_text(v, format!("project.todo.{id}.plan_md"))),
        status: rec.status,
        agent: rec.agent,
        executor_kind: rec.executor_kind,
        executor_ref: rec.executor_ref,
        active_session_id: rec.active_session_id,
        created_at: rec.created_at,
        updated_at: rec.updated_at,
    }))
}

/// Atomic board move: every id must exist under the expected initiative,
/// then positions become `(index+1)*1000` in the destination lane.
pub fn reorder(
    store: &rdb::store::Store,
    initiative_id: Option<&str>,
    board_status: &str,
    ids: &[String],
    now_ms: i64,
) -> Result<()> {
    anyhow::ensure!(
        !ids.is_empty() && ids.len() <= 1000,
        "invalid TODO reorder size"
    );
    let mut seen = std::collections::HashSet::new();
    anyhow::ensure!(ids.iter().all(|id| seen.insert(id)), "duplicate TODO id");
    let mut rows = Vec::with_capacity(ids.len());
    for id in ids {
        let rec = get(store, id)?;
        anyhow::ensure!(
            rec.as_ref()
                .is_some_and(|rec| rec.initiative_id.as_deref() == initiative_id),
            "TODO reorder contains missing id"
        );
        rows.push(rec.expect("checked above"));
    }
    let mut batch = rocksdb::WriteBatch::default();
    for (index, mut rec) in rows.into_iter().enumerate() {
        rec.board_status = board_status.to_owned();
        rec.position = (index as i64 + 1) * 1000;
        rec.updated_at = now_ms;
        tx::put(&mut batch, &todo_key(&rec.id), &rec)?;
    }
    tx::commit(store, batch)
}
