//! Integration tests for the brain (project goals / capability library)
//! persistence in `opencoder-store`:
//! - create → get (ordered eng_inputs) → list (newest first)
//! - update replaces the eng_inputs set (old rows disappear)
//! - delete cascades eng_inputs and vectors away
//! - upsert_vector is idempotent (PK replace, no duplicate rows)
//! - `vector_distance_cos` ordering with hand-built LE-f32 blob embeddings
//! - model-scoped search (vectors of other models never leak in)
//! - combined single-transaction create/update with vector: capability,
//!   eng_inputs and embedding commit together; update replaces the vector
//! - v14 → v15 migration creates the three brain tables
//! - playbooks (v25): save/get/list/delete round-trip, origin-scoped
//!   latest-by-digest, upsert keeps created_at, v24 → v25 migration

use opencoder_store::{
    BrainCapabilityRecord, BrainEngInputRecord, BrainVectorWrite, LibsqlStore, Store,
};
use tempfile::TempDir;

fn cap(id: &str, capability_type: &str, created_at: i64) -> BrainCapabilityRecord {
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

fn eng(capability_id: &str, content: &str, position: i64) -> BrainEngInputRecord {
    BrainEngInputRecord {
        id: None,
        capability_id: capability_id.into(),
        content: content.into(),
        position,
    }
}

// Little-endian f32 bytes — the storage encoding of brain_vectors.emb and the
// binding format verified to work with `vector32(?)`.
fn le(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

async fn scalar_i64(store: &LibsqlStore, sql: &str) -> i64 {
    let conn = store.conn().await.unwrap();
    let stmt = conn.prepare(sql).await.unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
}

// Precomputed embedding payload handed to the combined store methods —
// mirrors what the brain runtime does (embed first, persist second).
fn vec_write(dim: i64, model: &str, v: &[f32], embedded_at: i64) -> BrainVectorWrite {
    BrainVectorWrite {
        dim,
        model: model.into(),
        emb: le(v),
        embedded_at,
    }
}

// v14 → v15: a hand-built v14 database gains the three brain tables on
// reopen (bootstrap → migrate), and the brain API round-trips through the
// migrated schema. Mirrors the hand-written-old-schema pattern in
// store_migrations.rs.

#[path = "brain_store/suite_1.rs"]
mod suite_1;
#[path = "brain_store/suite_2.rs"]
mod suite_2;
