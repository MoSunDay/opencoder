//! Pure SQL assignments for TODO patches.
use libsql::Value;

/// The `SET` clause fragments + bound values shared by `patch_todo` and its
/// expected-status CAS variant — pure projection of the patch's `Some`
/// fields, no I/O. `Option<Option<String>>` fields distinguish "leave
/// unchanged" (outer `None`) from "clear to NULL" (`Some(None)`).
pub(super) fn todo_set_fragment(
    patch: &crate::project_types::ProjectTodoPatch,
) -> (Vec<&'static str>, Vec<Value>) {
    let mut sets: Vec<&'static str> = Vec::new();
    let mut vals: Vec<Value> = Vec::new();
    if let Some(v) = patch.title.as_deref() {
        sets.push("title = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.draft.as_deref() {
        sets.push("draft = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.plan_md.as_ref() {
        sets.push("plan_md = ?");
        vals.push(v.as_deref().into()); // Some(None) -> NULL
    }
    if let Some(v) = patch.status {
        sets.push("status = ?");
        vals.push(v.as_str().into());
    }
    if let Some(v) = patch.board_status.as_deref() {
        sets.push("board_status = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.position {
        sets.push("position = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.capability_id.as_ref() {
        sets.push("capability_id = ?");
        vals.push(v.as_deref().into());
    }
    if let Some(v) = patch.agent.as_deref() {
        sets.push("agent = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.executor_kind {
        sets.push("executor_kind = ?");
        vals.push(v.as_str().into());
    }
    if let Some(v) = patch.executor_ref.as_ref() {
        sets.push("executor_ref = ?");
        vals.push(v.as_deref().into()); // Some(None) -> NULL
    }
    if let Some(v) = patch.executor_spec.as_ref() {
        sets.push("executor_spec = ?");
        vals.push(v.as_deref().into()); // Some(None) -> NULL
    }
    if let Some(v) = patch.initiative_id.as_ref() {
        sets.push("initiative_id = ?");
        vals.push(v.as_deref().into());
    }
    if let Some(v) = patch.active_session_id.as_ref() {
        sets.push("active_session_id = ?");
        vals.push(v.as_deref().into());
    }
    (sets, vals)
}
