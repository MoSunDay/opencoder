//! Brain capability-library contracts for the rdb (RocksDB) backend,
//! ported from the SQL suites (`tests/brain_store/**`,
//! `tests/brain_content_guard.rs`): capability CRUD with ordered exemplar
//! inputs, atomic combined create/update with vectors, guarded version
//! updates, idempotent vector upserts, cosine ranking and model-scoped
//! search.

#[path = "rdb_vectors/suite_capability.rs"]
mod suite_capability;
#[path = "rdb_vectors/suite_vector.rs"]
mod suite_vector;
#[path = "rdb_vectors/suite_vector_validation.rs"]
mod suite_vector_validation;

use opencoder_store::rdb_store::RdbStore;
use opencoder_store::{BrainCapabilityRecord, BrainEngInputRecord, BrainVectorWrite, Store};

pub(crate) async fn mem() -> RdbStore {
    RdbStore::open_memory().await.unwrap()
}

pub(crate) fn cap(id: &str, capability_type: &str, created_at: i64) -> BrainCapabilityRecord {
    BrainCapabilityRecord {
        id: id.into(),
        capability_type: capability_type.into(),
        summary: format!("{id} summary"),
        input_desc: format!("{id} input"),
        output_desc: format!("{id} output"),
        created_at,
        updated_at: created_at,
    }
}

pub(crate) fn eng(capability_id: &str, content: &str, position: i64) -> BrainEngInputRecord {
    BrainEngInputRecord {
        id: None,
        capability_id: capability_id.into(),
        content: content.into(),
        position,
    }
}

// Little-endian f32 bytes — the storage encoding of the embedding rows.
pub(crate) fn le(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

// Precomputed embedding payload handed to the combined store methods —
// mirrors what the brain runtime does (embed first, persist second).
pub(crate) fn vec_write(dim: i64, model: &str, v: &[f32], embedded_at: i64) -> BrainVectorWrite {
    BrainVectorWrite {
        dim,
        model: model.into(),
        emb: le(v),
        embedded_at,
    }
}

// Raw-bytes variant of `vec_write` for payloads that must NOT satisfy the
// vector contract (wrong length, torn tail, non-finite values) — the
// rejection-path fixtures.
pub(crate) fn vec_bytes(dim: i64, model: &str, emb: Vec<u8>, embedded_at: i64) -> BrainVectorWrite {
    BrainVectorWrite {
        dim,
        model: model.into(),
        emb,
        embedded_at,
    }
}
