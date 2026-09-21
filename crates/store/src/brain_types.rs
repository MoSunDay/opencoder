use serde::{Deserialize, Serialize};

/// A durable capability in the project "brain" — the accumulated library of
/// goals/abilities an agent discovers while working in a repository. Pure
/// data: the brain runtime lives above the Store, which only persists it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainCapabilityRecord {
    pub id: String,
    pub capability_type: String,
    pub summary: String,
    pub input_desc: String,
    pub output_desc: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One exemplar input that exercises a capability (few-shot / replay material).
/// `id` is the autoincrement row id assigned by the store; `None` on insert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainEngInputRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub capability_id: String,
    pub content: String,
    pub position: i64,
}

/// Capability plus its ordered exemplar inputs — the read-model handed to the
/// brain runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainCapabilityDetail {
    pub capability: BrainCapabilityRecord,
    pub eng_inputs: Vec<BrainEngInputRecord>,
}

/// One nearest-neighbour hit from vector search over `brain_vectors`.
/// `distance` is the cosine distance (`vector_distance_cos`) between the
/// stored embedding and the query embedding (0 = identical direction).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainVectorHit {
    pub capability: BrainCapabilityRecord,
    pub distance: f64,
}

/// Precomputed embedding payload for one capability — the runtime embeds
/// BEFORE persisting, then hands these bytes to the combined store methods
/// so the capability row, its exemplar inputs and its vector commit (or
/// roll back) together in a single transaction.
#[derive(Debug, Clone)]
pub struct BrainVectorWrite {
    /// Vector dimension (emb.len() / 4).
    pub dim: i64,
    /// Embedding model the vector belongs to (search is model-scoped).
    pub model: String,
    /// Little-endian f32 bytes — the blob encoding `vector32()` accepts.
    pub emb: Vec<u8>,
    /// Millisecond timestamp for the vector row's updated_at.
    pub embedded_at: i64,
}
