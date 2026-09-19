//! Brain runtime — orchestrates `Store` persistence and `ChatStream`
//! embeddings around the pure `domain` functions. A data struct with
//! associated functions: no interior mutability, every method takes its
//! inputs by argument and returns its outputs by value.

use std::sync::Arc;

use anyhow::{bail, Context, Result};

use opencoder_core::brain::{BrainSchedulerContext, BrainSchedulerDecision};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, Message, RequestPurpose};
use opencoder_store::{
    BrainCapabilityDetail, BrainCapabilityRecord, BrainEngInputRecord, BrainPlaybookRecord,
    BrainVectorHit, BrainVectorWrite, Store,
};

use crate::domain;
use crate::error::{BrainNotFound, EmbeddingFailed};
use crate::playbook::{PlaybookInput, PlaybookSpec};
use crate::types::CapabilityInput;

/// Prefix for every persisted capability id (`brain-{ULID}`) — ULID body keeps
/// ids sortable and collision-free, mirroring the `todo-` id style.
pub const ID_PREFIX: &str = "brain";

/// Prefix for every persisted decision-tree plan id (`brain-plan-{ULID}`).
pub const PLAN_ID_PREFIX: &str = "brain-plan";

/// Prefix for every persisted playbook id (`playbook-{ULID}`).
pub const PLAYBOOK_ID_PREFIX: &str = "playbook";

/// Data struct of Arcs + strings: cloning shares the store/client handles
/// (cheap) so the web layer can hand the same runtime to the project module
/// while keeping its own copy in `AppState`.
#[derive(Clone)]
pub struct Runtime {
    pub(crate) store: Arc<dyn Store>,
    pub(crate) client: Arc<dyn ChatStream>,
    pub(crate) model: String,
    /// Chat model the dynamic planner prompts under (the framework-prompt
    /// LLM call in `planning.rs`). Defaults to the embedding model id; the
    /// production wiring overrides it with the config's small model.
    pub(crate) chat_model: String,
}

impl Runtime {
    pub fn new(
        store: Arc<dyn Store>,
        client: Arc<dyn ChatStream>,
        model: impl Into<String>,
    ) -> Self {
        let model = model.into();
        Self {
            chat_model: model.clone(),
            store,
            client,
            model,
        }
    }

    /// Override the planner chat model (builder; see `chat_model`).
    pub fn with_chat_model(mut self, model: impl Into<String>) -> Self {
        self.chat_model = model.into();
        self
    }

    /// The chat model the dynamic planner prompts under.
    pub fn chat_model(&self) -> &str {
        &self.chat_model
    }

    /// The embedding model every vector write/search is scoped to. Exposed so
    /// the web layer can log or validate it without re-deriving it.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// One bounded model turn for a v3 scheduler. The caller supplies only
    /// capability descriptors and execution references; detailed child data
    /// remains behind the execution gateway.
    pub async fn scheduler_decide(
        &self,
        context: &BrainSchedulerContext,
    ) -> Result<BrainSchedulerDecision> {
        anyhow::ensure!(
            context.schema_version == 3,
            "scheduler requires schema_version 3"
        );
        let payload = serde_json::to_string(context)?;
        anyhow::ensure!(
            payload.len() <= 512 * 1024,
            "scheduler context exceeds 512 KiB"
        );
        let mut stream = self.client.chat_stream(ChatRequest {
            purpose: RequestPurpose::Planning,
            model: self.chat_model.clone(),
            messages: vec![
                Message::system("brain-scheduler-v3", crate::scheduler::PROMPT),
                Message::user("scheduler-context", payload),
            ],
            tools: vec![],
            tool_choice: None,
            temperature: Some(0.0),
            max_tokens: Some(16_384),
            reasoning_effort: None,
            cache_salt: None,
        })?;
        while let Some(event) = stream.recv().await {
            match event {
                LlmEvent::Completed { text, .. } => {
                    anyhow::ensure!(
                        text.len() <= 256 * 1024,
                        "scheduler decision exceeds 256 KiB"
                    );
                    return Ok(serde_json::from_str(text.trim())?);
                }
                LlmEvent::Error(error) => anyhow::bail!("scheduler provider: {error}"),
                _ => {}
            }
        }
        anyhow::bail!("scheduler stream ended without completion")
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

    // ---- Playbooks (dual-track scheduling: fixed + LLM-generated graphs).
    // Pure store calls — no embeddings involved.

    /// Validate + persist a fresh fixed playbook. The id is minted here
    /// (`playbook-{ULID}`); every validation problem is aggregated into one
    /// joined error so callers see the complete report.
    pub async fn create_playbook(
        &self,
        _input: &PlaybookInput,
        _now_ms: i64,
    ) -> Result<PlaybookSpec> {
        anyhow::bail!(crate::graph::MIGRATION)
    }

    /// Replace an existing playbook's content. The id, origin, situation
    /// digest and `created_at` are preserved; name, trigger and steps come
    /// from the input. `Ok(None)` for an unknown id.
    pub async fn update_playbook(
        &self,
        _id: &str,
        _input: &PlaybookInput,
        _now_ms: i64,
    ) -> Result<Option<PlaybookSpec>> {
        anyhow::bail!(crate::graph::MIGRATION)
    }

    /// Fetch one persisted playbook record (`None` if absent).
    pub async fn get_playbook(&self, id: &str) -> Result<Option<BrainPlaybookRecord>> {
        self.store.get_brain_playbook(id).await
    }

    /// Fetch one playbook's decoded spec (`None` if absent; a corrupt stored
    /// spec is an error naming the id).
    pub async fn get_playbook_spec(&self, id: &str) -> Result<Option<PlaybookSpec>> {
        match self.get_playbook(id).await? {
            None => Ok(None),
            Some(record) => Ok(Some(
                serde_json::from_str(&record.spec_json)
                    .with_context(|| format!("stored playbook {id} spec is corrupt"))?,
            )),
        }
    }

    /// Every persisted playbook, newest first.
    pub async fn list_playbooks(&self) -> Result<Vec<BrainPlaybookRecord>> {
        self.store.list_brain_playbooks().await
    }

    /// Delete one playbook; `true` when a row was removed.
    pub async fn delete_playbook(&self, _id: &str) -> Result<bool> {
        anyhow::bail!(crate::graph::MIGRATION)
    }

    /// Embed exactly one text. Every upstream failure class — an embed call
    /// error, a cardinality mismatch, an empty vector — is carried as the
    /// typed [`EmbeddingFailed`] marker (the upstream chain folded into
    /// `detail`), which the web layer maps to a 502 via `downcast_ref`.
    /// `pub` so the control plane's trigger scan reuses the same embedding
    /// path as capability search.
    pub fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let mut vecs = self.embed_many(&[text.to_string()])?;
        let emb = vecs.pop().expect("cardinality checked in embed_many");
        Ok(emb)
    }

    /// Batch embedding entry point — callers embedding several texts (e.g.
    /// the control-plane trigger scan) must use one round-trip instead of
    /// N+1 `embed_one` calls. Every upstream failure (HTTP error,
    /// cardinality mismatch, empty vector) is carried as the typed
    /// [`EmbeddingFailed`] marker; on success every returned vector is
    /// non-empty.
    pub fn embed_many(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let vecs = match self.client.embed(texts, &self.model) {
            Ok(vecs) => vecs,
            Err(e) => {
                return Err(anyhow::Error::new(EmbeddingFailed {
                    detail: format!("{e:#}"),
                }));
            }
        };
        if vecs.len() != texts.len() {
            return Err(anyhow::Error::new(EmbeddingFailed {
                detail: format!("expected {} vectors, got {}", texts.len(), vecs.len()),
            }));
        }
        if vecs.iter().any(|v| v.is_empty()) {
            return Err(anyhow::Error::new(EmbeddingFailed {
                detail: "model returned an empty vector".to_string(),
            }));
        }
        Ok(vecs)
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
