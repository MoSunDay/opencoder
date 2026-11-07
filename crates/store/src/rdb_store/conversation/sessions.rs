//! Session root rows (`ses/<id>`), patches and cascading deletes.
//!
//! The row JSON is the `SessionMeta` plus the private `harness_runtime`
//! column, matching the SQL schema one-to-one. Child ranges (messages,
//! events, inputs) live under the session's prefix, so deleting a session
//! is a range delete per family plus index cleanup — the KV form of
//! `ON DELETE CASCADE`.

use anyhow::{bail, Result};
use opencoder_core::harness::HarnessRuntime;
use serde::{Deserialize, Serialize};

use crate::rdb_store::kv::{
    self, decode, event_prefix, get_json, input_prefix, message_prefix, session_key,
};
use crate::rdb_store::tx;
use crate::types::{SessionMeta, SessionPatch};

#[derive(Clone, Serialize, Deserialize)]
pub struct SessionRow {
    #[serde(flatten)]
    pub meta: SessionMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_runtime: Option<HarnessRuntime>,
}

impl From<SessionMeta> for SessionRow {
    fn from(meta: SessionMeta) -> Self {
        SessionRow {
            meta,
            harness_runtime: None,
        }
    }
}

/// `INSERT OR IGNORE`: creating an existing id is a no-op, never an error.
pub fn create(store: &rdb::store::Store, meta: &SessionMeta) -> Result<()> {
    let key = session_key(&meta.id);
    if rdb::store::get(store, b"", &key)
        .map_err(kv::engine_err)?
        .is_some()
    {
        return Ok(());
    }
    let mut stored = meta.clone();
    if stored.task_type.is_none() {
        stored.task_type = Some(crate::types::TASK_TYPE_PARENT.to_string());
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, &SessionRow::from(stored))?;
    // Inventory change log (SQL trigger on sessions INSERT).
    crate::rdb_store::execution::inventory::stage(store, &mut batch, &[("session", &meta.id)])?;
    tx::commit(store, batch)
}

pub fn get(store: &rdb::store::Store, id: &str) -> Result<Option<SessionMeta>> {
    Ok(get_json::<SessionRow>(store, &session_key(id))?.map(|r| r.meta))
}

pub fn row(store: &rdb::store::Store, id: &str) -> Result<Option<SessionRow>> {
    get_json(store, &session_key(id))
}

pub fn harness_runtime(store: &rdb::store::Store, id: &str) -> Result<Option<HarnessRuntime>> {
    Ok(row(store, id)?.and_then(|r| r.harness_runtime))
}

pub fn set_harness_runtime(
    store: &rdb::store::Store,
    id: &str,
    runtime: &HarnessRuntime,
) -> Result<()> {
    let key = session_key(id);
    let mut r = get_json::<SessionRow>(store, &key)?
        .ok_or_else(|| anyhow::anyhow!("session missing while saving harness runtime"))?;
    r.harness_runtime = Some(runtime.clone());
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, &r)?;
    tx::commit(store, batch)
}

/// Bump `updated_at` to `MAX(updated_at, ts)` inside the caller's batch
/// (appending a message IS session activity).
pub fn touch_activity(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    id: &str,
    ts: i64,
) -> Result<()> {
    let key = session_key(id);
    if let Some(mut r) = get_json::<SessionRow>(store, &key)? {
        if r.meta.updated_at < ts {
            r.meta.updated_at = ts;
        }
        tx::put(batch, &key, &r)?;
    }
    Ok(())
}

pub fn update(store: &rdb::store::Store, id: &str, patch: &SessionPatch) -> Result<()> {
    validate_patch(patch)?;
    let key = session_key(id);
    // A missing row matches the SQL UPDATE's 0-affected-rows no-op.
    let Some(mut r) = get_json::<SessionRow>(store, &key)? else {
        return Ok(());
    };
    let old_title = r.meta.title.clone();
    apply_patch(&mut r.meta, patch);
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, &r)?;
    // Inventory change log (SQL trigger: UPDATE OF title only fires when the
    // tracked column actually changed; no other patch field is tracked).
    if r.meta.title != old_title {
        crate::rdb_store::execution::inventory::stage(store, &mut batch, &[("session", id)])?;
    }
    tx::commit(store, batch)
}

fn validate_patch(p: &SessionPatch) -> Result<()> {
    let exclusives = [
        (
            p.summary.is_some() || p.summary_seq.is_some() || p.summary_images.is_some(),
            p.clear_summary,
            "summary",
        ),
        (
            p.handoff_seq.is_some() || p.handoff_plan.is_some(),
            p.clear_handoff,
            "handoff",
        ),
        (p.skill.is_some(), p.clear_skill, "skill"),
        (p.agent.is_some(), p.clear_agent, "agent"),
        (p.model.is_some(), p.clear_model, "model"),
        (
            p.autopilot_mode.is_some(),
            p.clear_autopilot_mode,
            "autopilot_mode",
        ),
        (p.requirement.is_some(), p.clear_requirement, "requirement"),
    ];
    for (has_value, cleared, name) in exclusives {
        if has_value && cleared {
            bail!("SessionPatch: {name} field and clear_{name} are mutually exclusive");
        }
    }
    Ok(())
}

fn apply_patch(meta: &mut SessionMeta, p: &SessionPatch) {
    if let Some(v) = &p.title {
        meta.title = Some(v.clone());
    }
    if let Some(v) = &p.agent {
        meta.agent = Some(v.clone());
    }
    if let Some(v) = &p.model {
        meta.model = Some(v.clone());
    }
    if let Some(v) = &p.autopilot_mode {
        meta.autopilot_mode = Some(v.clone());
    }
    if let Some(v) = &p.summary {
        meta.summary = Some(v.clone());
    }
    if let Some(v) = p.summary_seq {
        meta.summary_seq = Some(v);
    }
    if let Some(v) = &p.summary_images {
        meta.summary_images = v.clone();
    }
    if let Some(v) = &p.handoff_seq {
        meta.handoff_seq = Some(*v);
    }
    if let Some(v) = &p.handoff_plan {
        meta.handoff_plan = Some(v.clone());
    }
    if p.clear_summary {
        meta.summary = None;
        meta.summary_seq = None;
        meta.summary_images = Vec::new();
    }
    if p.clear_handoff {
        meta.handoff_seq = None;
        meta.handoff_plan = None;
    }
    if let Some(v) = &p.skill {
        meta.skill = Some(v.clone());
    }
    if p.clear_skill {
        meta.skill = None;
    }
    if p.clear_agent {
        meta.agent = None;
    }
    if p.clear_model {
        meta.model = None;
    }
    if p.clear_autopilot_mode {
        meta.autopilot_mode = None;
    }
    if let Some(v) = p.updated_at {
        meta.updated_at = v;
    }
    if let Some(v) = &p.requirement {
        meta.requirement = Some(v.clone());
    }
    if p.clear_requirement {
        meta.requirement = None;
    }
}

/// Delete one session and every child row (messages, events, inputs with
/// their indexes, subagent tasks by parent or child).
pub fn delete(store: &rdb::store::Store, id: &str) -> Result<()> {
    // Inventory change log (SQL trigger on sessions DELETE): fires per row
    // actually removed, so a missing row logs nothing.
    let existed = get_json::<SessionRow>(store, &session_key(id))?.is_some();
    let mut batch = rocksdb::WriteBatch::default();
    batch.delete(session_key(id));
    tx::delete_prefix(store, &mut batch, &message_prefix(id))?;
    tx::delete_prefix(store, &mut batch, &event_prefix(id))?;
    delete_input_children(store, &mut batch, id)?;
    super::subagent_tasks::delete_for_session(store, &mut batch, id)?;
    crate::rdb_store::todo::workflows::delete_for_session(store, &mut batch, id)?;
    if existed {
        crate::rdb_store::execution::inventory::stage(store, &mut batch, &[("session", id)])?;
    }
    tx::commit(store, batch)
}

/// Delete every session except `keep`; returns the removed session count.
pub fn clear_others(store: &rdb::store::Store, keep: &str) -> Result<u64> {
    let mut ids = Vec::new();
    for (key, _) in tx::scan_prefix(store, b"ses/")? {
        if let Some(id) = id_from_key(&key) {
            if id != keep {
                ids.push(id);
            }
        }
    }
    for id in &ids {
        delete(store, id)?;
    }
    Ok(ids.len() as u64)
}

pub fn id_from_key(key: &[u8]) -> Option<String> {
    std::str::from_utf8(key)
        .ok()?
        .strip_prefix("ses/")
        .map(str::to_owned)
}

/// Delete a session's inputs plus the global-seq and dedup index entries.
pub(crate) fn delete_input_children(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    session_id: &str,
) -> Result<()> {
    for (key, value) in tx::scan_prefix(store, &input_prefix(session_id))? {
        batch.delete(&key);
        if let Ok(rec) = decode::<super::inputs::InputRow>(&value) {
            tx::del(batch, &kv::input_seq_index(rec.seq));
            tx::del(batch, &kv::input_dedup_index(session_id, &rec.id));
        }
    }
    Ok(())
}

/// Load every session row (listing input; ordering is applied by `list`).
pub fn all_rows(store: &rdb::store::Store) -> Result<Vec<SessionRow>> {
    let mut out = Vec::new();
    for (_, value) in tx::scan_prefix(store, b"ses/")? {
        out.push(decode::<SessionRow>(&value)?);
    }
    Ok(out)
}
