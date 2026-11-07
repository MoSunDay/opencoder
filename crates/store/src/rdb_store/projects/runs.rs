//! Project TODO run rows (`prn/<id>` + `idx/prn/todo/<todo>/<ver>`), final
//! write-backs and status transitions (bounded inspection and byte-chunked
//! text reads live in [`super::runs_inspect`]).

use anyhow::{Context, Result};

use crate::project_types::{
    ProjectTodoRunKind, ProjectTodoRunPatch, ProjectTodoRunRecord, ProjectTodoRunStatus,
    ProjectTodoStatus,
};
use crate::rdb_store::kv::get_json;
use crate::rdb_store::projects::keys::{run_index_key, run_key};
use crate::rdb_store::projects::todos;
use crate::rdb_store::{index, tx};

/// Stage the run row plus its `todo_id → version` membership index; the
/// duplicate-id check mirrors the SQL `INSERT` (PRIMARY KEY on id) failing.
pub fn stage(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    rec: &ProjectTodoRunRecord,
) -> Result<()> {
    anyhow::ensure!(
        get_json::<ProjectTodoRunRecord>(store, &run_key(&rec.id))?.is_none(),
        "insert project todo run: id {} already exists",
        rec.id
    );
    tx::put(batch, &run_key(&rec.id), rec)?;
    index::put(
        batch,
        &run_index_key(&rec.todo_id, rec.version),
        rec.id.as_bytes(),
    );
    Ok(())
}

pub fn create(store: &rdb::store::Store, rec: &ProjectTodoRunRecord) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    stage(store, &mut batch, rec)?;
    // Inventory change log (SQL trigger on project_todo_runs INSERT).
    crate::rdb_store::execution::inventory::stage(store, &mut batch, &[("project", &rec.id)])?;
    tx::commit(store, batch)
}

/// Pure projection of a patch's `Some` fields (runs have no `updated_at`).
pub fn apply_patch(rec: &mut ProjectTodoRunRecord, patch: &ProjectTodoRunPatch) {
    if let Some(v) = &patch.input_snapshot {
        rec.input_snapshot = Some(v.clone());
    }
    if let Some(v) = &patch.trace_manifest {
        rec.trace_manifest = Some(v.clone());
    }
    if let Some(v) = &patch.plan_md {
        rec.plan_md = Some(v.clone());
    }
    if let Some(v) = &patch.output_md {
        rec.output_md = Some(v.clone());
    }
    if let Some(v) = &patch.output_ref {
        rec.output_ref = Some(v.clone());
    }
    if let Some(v) = &patch.capability_id {
        rec.capability_id = Some(v.clone());
    }
    if let Some(v) = &patch.plan_id {
        rec.plan_id = Some(v.clone());
    }
    if let Some(v) = &patch.session_id {
        rec.session_id = Some(v.clone());
    }
    if let Some(v) = patch.status {
        rec.status = v;
    }
    if let Some(v) = patch.finished_at {
        rec.finished_at = Some(v);
    }
}

pub fn patch(
    store: &rdb::store::Store,
    id: &str,
    patch: &ProjectTodoRunPatch,
    _now_ms: i64,
) -> Result<bool> {
    let Some(mut rec) = get(store, id)? else {
        return Ok(false);
    };
    let old_status = rec.status;
    apply_patch(&mut rec, patch);
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &run_key(id), &rec)?;
    stage_status_change(store, &mut batch, id, old_status, rec.status)?;
    tx::commit(store, batch)?;
    Ok(true)
}

/// Expected-status CAS variant: a stale convergence must not relabel a row
/// the driver already closed. An unchanged patch still reports the match.
pub fn patch_when(
    store: &rdb::store::Store,
    id: &str,
    when: ProjectTodoRunStatus,
    patch: &ProjectTodoRunPatch,
    _now_ms: i64,
) -> Result<bool> {
    let Some(mut rec) = get(store, id)? else {
        return Ok(false);
    };
    if rec.status != when {
        return Ok(false);
    }
    let old_status = rec.status;
    apply_patch(&mut rec, patch);
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &run_key(id), &rec)?;
    stage_status_change(store, &mut batch, id, old_status, rec.status)?;
    tx::commit(store, batch)?;
    Ok(true)
}

/// Inventory change log hook: the SQL trigger fires on
/// `UPDATE OF id,todo_id,created_at,status`, but those columns are either
/// immutable after create or only ever change via `status` — so a status
/// transition is the only tracked change.
fn stage_status_change(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    id: &str,
    old_status: ProjectTodoRunStatus,
    new_status: ProjectTodoRunStatus,
) -> Result<()> {
    if old_status != new_status {
        crate::rdb_store::execution::inventory::stage(store, batch, &[("project", id)])?;
    }
    Ok(())
}

pub fn get(store: &rdb::store::Store, id: &str) -> Result<Option<ProjectTodoRunRecord>> {
    get_json(store, &run_key(id))
}

/// Terminal write-back in one batch with the todo status it drives:
/// Plan→Planned on Done, Step never touches the todo, Execute maps
/// Done/Cancelled/other onto Done/Planned/Failed.
pub fn finish(
    store: &rdb::store::Store,
    id: &str,
    patch: &ProjectTodoRunPatch,
    now_ms: i64,
) -> Result<bool> {
    let run = get(store, id)?.context("run missing during finalization")?;
    if run.status != ProjectTodoRunStatus::Running {
        return Ok(false);
    }
    let status = patch.status.context("final run status missing")?;
    anyhow::ensure!(
        status != ProjectTodoRunStatus::Running,
        "run finalization requires terminal status"
    );
    let next = if run.kind == ProjectTodoRunKind::Plan {
        (status == ProjectTodoRunStatus::Done).then_some(ProjectTodoStatus::Planned)
    } else if run.kind == ProjectTodoRunKind::Step {
        None
    } else {
        Some(match status {
            ProjectTodoRunStatus::Done => ProjectTodoStatus::Done,
            ProjectTodoRunStatus::Cancelled => ProjectTodoStatus::Planned,
            _ => ProjectTodoStatus::Failed,
        })
    };
    let mut batch = rocksdb::WriteBatch::default();
    if let Some(status) = next {
        let current =
            todos::get(store, &run.todo_id)?.context("todo missing during finalization")?;
        let todo_patch = crate::project_types::ProjectTodoPatch {
            status: Some(status),
            plan_md: if run.kind == ProjectTodoRunKind::Plan {
                Some(patch.output_md.clone())
            } else {
                None
            },
            ..Default::default()
        };
        if run.kind == ProjectTodoRunKind::Plan || current.status == ProjectTodoStatus::Running {
            let mut updated = current;
            todos::apply_patch(&mut updated, &todo_patch);
            updated.updated_at = now_ms;
            tx::put(
                &mut batch,
                &crate::rdb_store::projects::keys::todo_key(&run.todo_id),
                &updated,
            )?;
        }
    }
    let mut rec = run;
    apply_patch(&mut rec, patch);
    tx::put(&mut batch, &run_key(id), &rec)?;
    // Inventory change log: finish always transitions Running→terminal.
    crate::rdb_store::execution::inventory::stage(store, &mut batch, &[("project", id)])?;
    tx::commit(store, batch)?;
    Ok(true)
}
