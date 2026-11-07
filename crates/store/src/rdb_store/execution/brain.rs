//! Brain capability library (`brain_capabilities` / `brain_eng_inputs` /
//! `brain_vectors`) over the rdb KV engine.
//!
//! Key families:
//! - `cap/<id>`  capability root row
//! - `bei/<cap_id>/<row_id:020>`  exemplar-input rows; row ids come from the
//!   `seq/brain-eng-input` persisted counter (SQL AUTOINCREMENT) and reads
//!   order by `(position, row_id)` — the `ORDER BY position, id` contract
//! - `vec/<cap_id>`  the one embedding row of a capability (`capability_id`
//!   is `brain_vectors`' primary key, so an upsert replaces in place)
//!
//! Combined writes stage capability + inputs + vector in a single atomic
//! batch: a failed batch (duplicate capability id, guarded-version conflict)
//! commits nothing, which is the rollback half of the "no capability without
//! vector" invariant. Every embedding payload passes the same gate
//! ([`validate_vector`]: positive dim, exactly `dim * 4` bytes, finite
//! values) before anything is staged, and search skips stored rows that
//! fail the same checks instead of scoring them.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::rdb_store::kv::{self, pad_ord};
use crate::rdb_store::tx;
use crate::rdb_store::vector;
use crate::{
    BrainCapabilityDetail, BrainCapabilityRecord, BrainContentConflict, BrainEngInputRecord,
    BrainVectorHit, BrainVectorWrite,
};

/// Counter family backing exemplar-input row ids.
const ENG_COUNTER: &str = "brain-eng-input";

pub fn capability_key(id: &str) -> Vec<u8> {
    format!("cap/{id}").into_bytes()
}

pub fn eng_input_prefix(capability_id: &str) -> Vec<u8> {
    format!("bei/{capability_id}/").into_bytes()
}

pub fn eng_input_key(capability_id: &str, row_id: i64) -> Vec<u8> {
    let mut key = eng_input_prefix(capability_id);
    key.extend_from_slice(pad_ord(row_id).as_bytes());
    key
}

pub fn vector_key(capability_id: &str) -> Vec<u8> {
    format!("vec/{capability_id}").into_bytes()
}

/// One stored embedding (`brain_vectors` row).
#[derive(Clone, Serialize, Deserialize)]
struct VectorRow {
    dim: i64,
    model: String,
    emb: Vec<u8>,
    updated_at: i64,
}

/// INSERT a capability plus its exemplar inputs in one batch. Input `id`
/// fields are ignored; fresh ids are assigned (autoincrement semantics).
pub fn create(
    store: &rdb::store::Store,
    capability: &BrainCapabilityRecord,
    eng_inputs: &[BrainEngInputRecord],
) -> Result<()> {
    let key = capability_key(&capability.id);
    if kv::get_json::<BrainCapabilityRecord>(store, &key)?.is_some() {
        bail!("insert brain capability: UNIQUE constraint failed: brain_capabilities.id");
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, capability)?;
    stage_eng_inputs(store, &mut batch, eng_inputs)?;
    tx::commit(store, batch)
}

/// INSERT capability + exemplar inputs + embedding in ONE atomic batch —
/// the runtime path that cannot leave a capability without its vector.
pub fn create_with_vector(
    store: &rdb::store::Store,
    capability: &BrainCapabilityRecord,
    eng_inputs: &[BrainEngInputRecord],
    vector: &BrainVectorWrite,
) -> Result<()> {
    validate_vector(vector)?;
    let key = capability_key(&capability.id);
    if kv::get_json::<BrainCapabilityRecord>(store, &key)?.is_some() {
        bail!("insert brain capability: UNIQUE constraint failed: brain_capabilities.id");
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, capability)?;
    stage_eng_inputs(store, &mut batch, eng_inputs)?;
    stage_vector(&mut batch, &capability.id, vector)?;
    tx::commit(store, batch)
}

/// UPDATE every capability field and REPLACE the exemplar-input set; the
/// embedding row is left untouched. A missing row is the SQL UPDATE's
/// 0-affected-rows no-op (deviation: the SQL backend would only fail later
/// on the `brain_eng_inputs` foreign key when inputs are non-empty).
pub fn update(
    store: &rdb::store::Store,
    capability: &BrainCapabilityRecord,
    eng_inputs: &[BrainEngInputRecord],
) -> Result<()> {
    if kv::get_json::<BrainCapabilityRecord>(store, &capability_key(&capability.id))?.is_none() {
        return Ok(());
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &capability_key(&capability.id), capability)?;
    replace_eng_inputs(store, &mut batch, &capability.id, eng_inputs)?;
    tx::commit(store, batch)
}

/// Guarded combined update: compares the stored detail's `content_version`
/// against `expected` (when given) before replacing content + inputs +
/// vector in the same batch. A stale or missing row rejects with the typed
/// [`BrainContentConflict`] the API layer maps for its callers.
pub fn update_if_version(
    store: &rdb::store::Store,
    capability: &BrainCapabilityRecord,
    eng_inputs: &[BrainEngInputRecord],
    vector: &BrainVectorWrite,
    expected: Option<&str>,
) -> Result<()> {
    validate_vector(vector)?;
    let current = get(store, &capability.id)?;
    match expected {
        Some(expected) => {
            if current
                .as_ref()
                .map(BrainCapabilityDetail::content_version)
                .as_deref()
                != Some(expected)
            {
                return Err(BrainContentConflict {
                    id: capability.id.clone(),
                }
                .into());
            }
        }
        None => {
            // The vector row's INSERT OR REPLACE carries the capability FK,
            // so updating an unknown id cannot succeed in the SQL backend.
            if current.is_none() {
                bail!("update brain vector: FOREIGN KEY constraint failed");
            }
        }
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &capability_key(&capability.id), capability)?;
    replace_eng_inputs(store, &mut batch, &capability.id, eng_inputs)?;
    stage_vector(&mut batch, &capability.id, vector)?;
    tx::commit(store, batch)
}

/// DELETE a capability; its exemplar inputs and embedding row follow in the
/// same batch (the `ON DELETE CASCADE` equivalent).
pub fn delete(store: &rdb::store::Store, id: &str) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    tx::del(&mut batch, &capability_key(id));
    tx::delete_prefix(store, &mut batch, &eng_input_prefix(id))?;
    tx::del(&mut batch, &vector_key(id));
    tx::commit(store, batch)
}

/// Fetch one capability with its exemplar inputs ordered by `position, id`.
pub fn get(store: &rdb::store::Store, id: &str) -> Result<Option<BrainCapabilityDetail>> {
    let Some(capability) = kv::get_json::<BrainCapabilityRecord>(store, &capability_key(id))?
    else {
        return Ok(None);
    };
    Ok(Some(BrainCapabilityDetail {
        capability,
        eng_inputs: eng_inputs(store, id)?,
    }))
}

/// Every capability, newest first (`created_at DESC, id DESC`), inputs
/// attached.
pub fn list(store: &rdb::store::Store) -> Result<Vec<BrainCapabilityDetail>> {
    let mut details = Vec::new();
    for (_, value) in tx::scan_prefix(store, b"cap/")? {
        let capability: BrainCapabilityRecord = kv::decode(&value)?;
        details.push(BrainCapabilityDetail {
            capability,
            eng_inputs: Vec::new(),
        });
    }
    details.sort_by(|a, b| {
        let x = (&a.capability.created_at, &a.capability.id);
        let y = (&b.capability.created_at, &b.capability.id);
        y.cmp(&x)
    });
    for detail in &mut details {
        detail.eng_inputs = eng_inputs(store, &detail.capability.id)?;
    }
    Ok(details)
}

/// INSERT OR REPLACE the embedding of a capability. `capability_id` is the
/// row key, so a re-upsert overwrites in place — never duplicates.
pub fn upsert_vector(
    store: &rdb::store::Store,
    capability_id: &str,
    dim: i64,
    model: &str,
    emb: &[u8],
    updated_at: i64,
) -> Result<()> {
    let vector = BrainVectorWrite {
        dim,
        model: model.to_string(),
        emb: emb.to_vec(),
        embedded_at: updated_at,
    };
    validate_vector(&vector)?;
    if kv::get_json::<BrainCapabilityRecord>(store, &capability_key(capability_id))?.is_none() {
        bail!("upsert brain vector: FOREIGN KEY constraint failed");
    }
    let mut batch = rocksdb::WriteBatch::default();
    stage_vector(&mut batch, capability_id, &vector)?;
    tx::commit(store, batch)
}

/// Nearest-neighbour cosine-distance search over the stored embeddings,
/// scoped to one embedding model, ascending distance, capped at `limit`.
/// Brute force via [`vector::top_k`]: exact for the catalog sizes the
/// capability library holds. Vectors whose capability row is gone never
/// hit (the SQL JOIN's behavior), and rows whose metadata or blob fail
/// validation are skipped — search degrades to fewer hits, never panics
/// and never scores a torn blob as a shorter valid vector.
pub fn search(
    store: &rdb::store::Store,
    model: &str,
    query_emb: &[u8],
    limit: u32,
) -> Result<Vec<BrainVectorHit>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let query = vector::decode_f32_le(query_emb);
    let candidates: Vec<(String, Vec<f32>)> = tx::scan_prefix(store, b"vec/")?
        .into_iter()
        .filter_map(|(key, value)| searchable_row(&key, &value, model))
        .collect();
    let ranked = vector::top_k(
        candidates
            .iter()
            .map(|(id, emb)| (id.as_str(), emb.as_slice())),
        &query,
        limit as usize,
    );
    let mut hits = Vec::with_capacity(ranked.len());
    for (id, similarity) in ranked {
        if let Some(capability) =
            kv::get_json::<BrainCapabilityRecord>(store, &capability_key(&id))?
        {
            hits.push(BrainVectorHit {
                capability,
                distance: (1.0 - similarity) as f64,
            });
        }
    }
    Ok(hits)
}

/// Capability id carried by a `vec/<id>` row key.
fn id_from_key(key: &[u8]) -> Option<String> {
    std::str::from_utf8(key)
        .ok()?
        .strip_prefix("vec/")
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// A `vec/<id>` row turned into a searchable candidate: `Some` only when
/// the key parses, the row decodes, the model matches and the blob is
/// exactly `dim` finite f32 values ([`vector::decode_f32_le_exact`]).
/// Anything else — torn JSON metadata, a non-positive `dim`, a blob that
/// disagrees with it or non-finite values — is `None` and skipped.
fn searchable_row(key: &[u8], value: &[u8], model: &str) -> Option<(String, Vec<f32>)> {
    let id = id_from_key(key)?;
    let row: VectorRow = kv::decode(value).ok()?;
    if row.model != model {
        return None;
    }
    let dim = usize::try_from(row.dim).ok().filter(|d| *d > 0)?;
    let emb = vector::decode_f32_le_exact(&row.emb, dim)?;
    Some((id, emb))
}

/// Shared payload gate for every embedding write: `dim` positive, `emb`
/// exactly `dim * 4` bytes, all decoded values finite. Callers run it
/// before staging anything, so a rejected payload leaves no batch to
/// commit — the write-side half of "search never sees a torn blob".
fn validate_vector(payload: &BrainVectorWrite) -> Result<()> {
    if payload.dim <= 0 {
        bail!("brain vector: dim must be positive, got {}", payload.dim);
    }
    let Ok(dim) = usize::try_from(payload.dim) else {
        bail!(
            "brain vector: dim {} does not fit the platform word",
            payload.dim
        );
    };
    let Some(bytes) = dim.checked_mul(4) else {
        bail!("brain vector: dim {dim} overflows the blob size");
    };
    if payload.emb.len() != bytes {
        bail!(
            "brain vector: emb is {} bytes, dim {dim} needs exactly {bytes}",
            payload.emb.len()
        );
    }
    if vector::decode_f32_le(&payload.emb)
        .iter()
        .any(|v| !v.is_finite())
    {
        bail!("brain vector: emb must hold only finite f32 values (dim {dim})");
    }
    Ok(())
}

/// Stage the exemplar-input rows, assigning fresh global row ids.
///
/// Block allocation (one counter read for the whole set) because the shared
/// `seq::next_global` helper reads the committed counter, so repeated calls
/// inside one batch would hand out the same value; staging the counter write
/// together with the rows keeps the allocation atomic with them.
fn stage_eng_inputs(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    inputs: &[BrainEngInputRecord],
) -> Result<()> {
    let counter = kv::seq_counter_key(ENG_COUNTER);
    let current: i64 = match rdb::store::get(store, b"", &counter).map_err(kv::engine_err)? {
        Some(bytes) => kv::decode(&bytes)?,
        None => 0,
    };
    let mut next_id = current;
    for input in inputs {
        next_id += 1;
        let record = BrainEngInputRecord {
            id: Some(next_id),
            capability_id: input.capability_id.clone(),
            content: input.content.clone(),
            position: input.position,
        };
        tx::put(
            batch,
            &eng_input_key(&input.capability_id, next_id),
            &record,
        )?;
    }
    if next_id != current {
        tx::put(batch, &counter, &next_id)?;
    }
    Ok(())
}

/// Replace semantics: drop the capability's current input rows, then stage
/// the new set — removed inputs disappear atomically.
fn replace_eng_inputs(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    capability_id: &str,
    inputs: &[BrainEngInputRecord],
) -> Result<()> {
    tx::delete_prefix(store, batch, &eng_input_prefix(capability_id))?;
    stage_eng_inputs(store, batch, inputs)
}

fn stage_vector(
    batch: &mut rocksdb::WriteBatch,
    capability_id: &str,
    vector: &BrainVectorWrite,
) -> Result<()> {
    let row = VectorRow {
        dim: vector.dim,
        model: vector.model.clone(),
        emb: vector.emb.clone(),
        updated_at: vector.embedded_at,
    };
    tx::put(batch, &vector_key(capability_id), &row)
}

/// Inputs of one capability ordered by `position, id`.
fn eng_inputs(store: &rdb::store::Store, capability_id: &str) -> Result<Vec<BrainEngInputRecord>> {
    let mut rows: Vec<BrainEngInputRecord> = Vec::new();
    for (_, value) in tx::scan_prefix(store, &eng_input_prefix(capability_id))? {
        rows.push(kv::decode(&value)?);
    }
    rows.sort_by_key(|r| (r.position, r.id.unwrap_or(0)));
    Ok(rows)
}
