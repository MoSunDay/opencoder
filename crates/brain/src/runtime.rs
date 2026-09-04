//! Brain runtime — orchestrates `Store` persistence and `ChatStream`
//! embeddings around the pure `domain` functions. A data struct with
//! associated functions: no interior mutability, every method takes its
//! inputs by argument and returns its outputs by value.

use std::sync::Arc;

use anyhow::{bail, Context, Result};

use opencoder_llm::ChatStream;
use opencoder_store::{
    BrainCapabilityDetail, BrainCapabilityRecord, BrainEngInputRecord, BrainVectorHit,
    BrainVectorWrite, Store,
};

use crate::domain;
use crate::error::{BrainNotFound, EmbeddingFailed};
use crate::types::CapabilityInput;

/// Prefix for every persisted capability id (`brain-{ULID}`) — ULID body keeps
/// ids sortable and collision-free, mirroring the `todo-` id style.
pub const ID_PREFIX: &str = "brain";

pub struct Runtime {
    store: Arc<dyn Store>,
    client: Arc<dyn ChatStream>,
    model: String,
}

impl Runtime {
    pub fn new(
        store: Arc<dyn Store>,
        client: Arc<dyn ChatStream>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            store,
            client,
            model: model.into(),
        }
    }

    /// The embedding model every vector write/search is scoped to. Exposed so
    /// the web layer can log or validate it without re-deriving it.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Validate → compose → embed → persist (capability row, exemplar inputs
    /// and embedding commit in ONE store transaction) → return the stored
    /// detail. The id is minted here.
    pub async fn upsert_capability(
        &self,
        input: &CapabilityInput,
        now_ms: i64,
    ) -> Result<BrainCapabilityDetail> {
        domain::validate(input)?;
        let emb = self.embed_one(&domain::compose_embed_text(input))?;
        let id = format!("{ID_PREFIX}-{}", ulid::Ulid::new());
        let record = capability_record(&id, input, now_ms, now_ms);
        let eng_inputs = eng_input_records(&id, input);
        let vector = BrainVectorWrite {
            dim: emb.len() as i64,
            model: self.model.clone(),
            emb: domain::f32_slice_to_le_bytes(&emb),
            embedded_at: now_ms,
        };
        self.store
            .create_brain_capability_with_vector(&record, &eng_inputs, &vector)
            .await?;
        self.get_capability(&id)
            .await?
            .with_context(|| format!("brain capability not found after insert: {id}"))
    }

    /// Replace an existing capability's content and re-embed it; the
    /// capability row, its exemplar inputs and the fresh embedding replace
    /// atomically in ONE store transaction (a failed update can never leave
    /// new content answering search with a stale old vector). `created_at`
    /// is preserved from the stored row; only content and `updated_at` move.
    /// An unknown id fails as the typed [`crate::error::BrainNotFound`] marker.
    pub async fn update_capability(
        &self,
        id: &str,
        input: &CapabilityInput,
        now_ms: i64,
    ) -> Result<BrainCapabilityDetail> {
        domain::validate(input)?;
        // Unknown id is a typed 404-class marker (the web layer downcasts on
        // the type); the POST-write contexts below ("not found after
        // insert/update") are invariant violations and stay plain anyhow
        // strings so they remain 500-class.
        let existing = self
            .store
            .get_brain_capability(id)
            .await?
            .ok_or_else(|| anyhow::Error::new(BrainNotFound { id: id.to_string() }))?;
        let emb = self.embed_one(&domain::compose_embed_text(input))?;
        let record = capability_record(id, input, existing.capability.created_at, now_ms);
        let eng_inputs = eng_input_records(id, input);
        let vector = BrainVectorWrite {
            dim: emb.len() as i64,
            model: self.model.clone(),
            emb: domain::f32_slice_to_le_bytes(&emb),
            embedded_at: now_ms,
        };
        self.store
            .update_brain_capability_with_vector(&record, &eng_inputs, &vector)
            .await?;
        self.get_capability(id)
            .await?
            .with_context(|| format!("brain capability not found after update: {id}"))
    }

    /// Delete a capability; exemplar inputs and its embedding cascade.
    pub async fn delete_capability(&self, id: &str) -> Result<()> {
        self.store.delete_brain_capability(id).await
    }

    /// Fetch one capability with its ordered exemplar inputs (`None` if absent).
    pub async fn get_capability(&self, id: &str) -> Result<Option<BrainCapabilityDetail>> {
        self.store.get_brain_capability(id).await
    }

    /// Fetch every capability (newest first) with its exemplar inputs.
    pub async fn list_capabilities(&self) -> Result<Vec<BrainCapabilityDetail>> {
        self.store.list_brain_capabilities().await
    }

    /// Nearest-neighbour search over stored capability embeddings, scoped to
    /// this runtime's embedding model, ascending by cosine distance.
    pub async fn search(&self, query: &str, k: u32) -> Result<Vec<BrainVectorHit>> {
        let query = query.trim();
        if query.is_empty() {
            bail!("search query must not be empty");
        }
        let emb = self.embed_one(query)?;
        self.store
            .search_brain_vectors(&self.model, &domain::f32_slice_to_le_bytes(&emb), k)
            .await
    }

    /// Embed exactly one text. Every upstream failure class — an embed call
    /// error, a cardinality mismatch, an empty vector — is carried as the
    /// typed [`EmbeddingFailed`] marker (the upstream chain folded into
    /// `detail`), which the web layer maps to a 502 via `downcast_ref`.
    fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let mut vecs = match self.client.embed(&[text.to_string()], &self.model) {
            Ok(vecs) => vecs,
            Err(e) => {
                return Err(anyhow::Error::new(EmbeddingFailed {
                    detail: format!("{e:#}"),
                }))
            }
        };
        if vecs.len() != 1 {
            return Err(anyhow::Error::new(EmbeddingFailed {
                detail: format!("expected 1 vector, got {}", vecs.len()),
            }));
        }
        let emb = vecs.pop().expect("length checked above");
        if emb.is_empty() {
            return Err(anyhow::Error::new(EmbeddingFailed {
                detail: "model returned an empty vector".to_string(),
            }));
        }
        Ok(emb)
    }
}

/// Build the persistence record for a payload. Text fields are normalized
/// (trimmed) exactly the way `domain::validate` checks them.
fn capability_record(
    id: &str,
    input: &CapabilityInput,
    created_at: i64,
    updated_at: i64,
) -> BrainCapabilityRecord {
    BrainCapabilityRecord {
        id: id.to_string(),
        capability_type: input.capability_type.trim().to_string(),
        summary: input.summary.trim().to_string(),
        input_desc: input.input_desc.trim().to_string(),
        output_desc: input.output_desc.trim().to_string(),
        created_at,
        updated_at,
    }
}

/// Exemplar inputs with `position` = original order; the store preserves it.
fn eng_input_records(id: &str, input: &CapabilityInput) -> Vec<BrainEngInputRecord> {
    input
        .eng_inputs
        .iter()
        .enumerate()
        .map(|(i, content)| BrainEngInputRecord {
            id: None,
            capability_id: id.to_string(),
            content: content.trim().to_string(),
            position: i as i64,
        })
        .collect()
}
