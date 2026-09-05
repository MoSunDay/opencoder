//! Integration tests for the brain runtime: real `LibsqlStore` (in-memory)
//! plus `MockChatClient` — zero tokens, zero network.
//!
//! The mock embedder is a pure hash, so embedding the *exact same text*
//! yields distance 0; each test exploits that to predict search outcomes
//! deterministically.

use std::sync::Arc;

use anyhow::{bail, Result};

use opencoder_brain::domain;
use opencoder_brain::{CapabilityInput, Runtime};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use tokio::sync::mpsc;

const MODEL: &str = "mock-embed-v1";

async fn setup() -> (Runtime, Arc<MockChatClient>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(MockChatClient::new());
    let client: Arc<dyn ChatStream> = mock.clone();
    (Runtime::new(store, client, MODEL), mock)
}

fn payload(summary: &str) -> CapabilityInput {
    CapabilityInput {
        capability_type: "tool-usage".into(),
        summary: summary.into(),
        input_desc: "a failing test id".into(),
        output_desc: "a green test run".into(),
        eng_inputs: vec![
            "fix the login test".into(),
            "refactor the auth module".into(),
        ],
    }
}

#[tokio::test]
async fn upsert_then_get_list_and_self_search_roundtrip() {
    let (rt, mock) = setup().await;
    assert_eq!(rt.model(), MODEL);
    let input = payload("can repair failing rust tests");

    let created = rt.upsert_capability(&input, 1_000).await.unwrap();
    assert!(created.capability.id.starts_with("brain-"));
    assert_eq!(created.capability.created_at, 1_000);
    assert_eq!(created.eng_inputs.len(), 2);
    assert_eq!(created.eng_inputs[0].content, "fix the login test");
    assert_eq!(created.eng_inputs[0].position, 0);
    assert_eq!(created.eng_inputs[1].content, "refactor the auth module");
    assert_eq!(created.eng_inputs[1].position, 1);

    let detail = rt
        .get_capability(&created.capability.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.capability.summary, "can repair failing rust tests");
    assert_eq!(
        detail
            .eng_inputs
            .iter()
            .map(|r| (r.content.as_str(), r.position))
            .collect::<Vec<_>>(),
        vec![("fix the login test", 0), ("refactor the auth module", 1)]
    );

    let list = rt.list_capabilities().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].capability.id, created.capability.id);

    // The capability's own embed text must retrieve it with distance 0.
    let hits = rt
        .search(&domain::compose_embed_text(&input), 5)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].capability.id, created.capability.id);
    assert!(
        hits[0].distance.abs() < 1e-6,
        "distance {}",
        hits[0].distance
    );

    // Every embedding went through the client, one text at a time, on-model.
    let calls = mock.embed_calls();
    assert!(calls.len() >= 2); // 1 upsert + 1 search
    assert!(calls.iter().all(|(t, m)| t.len() == 1 && m == MODEL));
}

#[tokio::test]
async fn update_reembeds_so_old_text_no_longer_wins() {
    let (rt, _) = setup().await;
    let old_a = payload("write new rust modules from scratch");
    rt.upsert_capability(&payload("walk a nervous dog calmly"), 1_000)
        .await
        .unwrap();
    let a = rt.upsert_capability(&old_a, 1_100).await.unwrap();

    let old_text = domain::compose_embed_text(&old_a);
    let hits = rt.search(&old_text, 5).await.unwrap();
    assert_eq!(hits[0].capability.id, a.capability.id); // A wins before update

    let new_a = payload("author brand new rust modules quickly");
    let updated = rt
        .update_capability(&a.capability.id, &new_a, 9_999)
        .await
        .unwrap();
    assert_eq!(updated.capability.summary, new_a.summary);
    assert_eq!(updated.capability.created_at, a.capability.created_at); // preserved
    assert_eq!(updated.capability.updated_at, 9_999);
    assert_eq!(updated.eng_inputs.len(), 2); // replace semantics kept them

    // Old text no longer ranks A first...
    let hits = rt.search(&old_text, 5).await.unwrap();
    assert_ne!(hits[0].capability.id, a.capability.id);
    // ...while the new embed text hits A exactly.
    let hits = rt
        .search(&domain::compose_embed_text(&new_a), 5)
        .await
        .unwrap();
    assert_eq!(hits[0].capability.id, a.capability.id);
    assert!(
        hits[0].distance.abs() < 1e-6,
        "distance {}",
        hits[0].distance
    );
}

#[tokio::test]
async fn delete_removes_capability_and_vector() {
    let (rt, _) = setup().await;
    let input = payload("triage ci failures overnight");
    let created = rt.upsert_capability(&input, 1_000).await.unwrap();

    rt.delete_capability(&created.capability.id).await.unwrap();
    assert!(rt
        .get_capability(&created.capability.id)
        .await
        .unwrap()
        .is_none());
    assert!(rt.list_capabilities().await.unwrap().is_empty());
    let hits = rt
        .search(&domain::compose_embed_text(&input), 5)
        .await
        .unwrap();
    assert!(hits.is_empty());
}

#[tokio::test]
async fn invalid_payload_never_reaches_the_store() {
    let (rt, mock) = setup().await;
    let mut input = payload("never persisted");
    input.summary = "   ".into();

    let err = rt
        .upsert_capability(&input, 1_000)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("summary"), "got: {err}");
    assert!(rt.list_capabilities().await.unwrap().is_empty());
    assert!(mock.embed_calls().is_empty()); // failed before any embedding call
}

#[tokio::test]
async fn search_respects_k_and_orders_by_distance() {
    let (rt, _) = setup().await;
    let texts = [
        payload("rotate tls certificates safely"),
        payload("rewrite the parser grammar"),
        payload("shard the event log table"),
    ];
    let mut ids = Vec::new();
    for (i, input) in texts.iter().enumerate() {
        ids.push(
            rt.upsert_capability(input, 1_000 + i as i64)
                .await
                .unwrap()
                .capability
                .id,
        );
    }

    let hits = rt
        .search(&domain::compose_embed_text(&texts[1]), 2)
        .await
        .unwrap();
    assert_eq!(hits.len(), 2); // k truncated 3 → 2
    assert_eq!(hits[0].capability.id, ids[1]); // best match is itself
    assert!(hits[0].distance < hits[1].distance); // ascending order
}

#[tokio::test]
async fn update_unknown_id_fails() {
    let (rt, _) = setup().await;
    let err = rt
        .update_capability("brain-nope", &payload("ghost capability"), 1_000)
        .await
        .unwrap_err();
    // Typed marker, not a plain context: the web layer downcasts on the type.
    assert!(
        err.downcast_ref::<opencoder_brain::BrainNotFound>()
            .is_some(),
        "must be the typed BrainNotFound marker, got: {err:#}"
    );
    // Display keeps the historical body shape the HTTP 404 asserts on.
    assert_eq!(err.to_string(), "brain capability not found: brain-nope");
    assert!(rt.list_capabilities().await.unwrap().is_empty());
}

// ─── upstream embed failures → typed EmbeddingFailed marker, zero residue ──

/// How the fake client below breaks `embed` — one variant per failure class
/// `Runtime::embed_one` must fold into the typed marker.
#[derive(Debug, Clone, Copy)]
enum BreakEmbed {
    /// `embed` itself errors (HTTP outage, auth failure, …).
    Bail,
    /// `embed` answers with the wrong number of vectors.
    Cardinality,
    /// `embed` answers with a single zero-dimension vector.
    EmptyVector,
}

/// Minimal `ChatStream` whose `embed` always misbehaves per [`BreakEmbed`] —
/// same shape as the web layer's bail-only `UnavailableClient`, but
/// configurable so all three failure classes are covered.
struct BrokenEmbedClient {
    mode: BreakEmbed,
}

impl ChatStream for BrokenEmbedClient {
    fn chat_stream(&self, _req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        bail!("llm endpoint unavailable")
    }

    fn backend(&self) -> &'static str {
        "broken-embed"
    }

    fn embed(&self, _texts: &[String], _model: &str) -> Result<Vec<Vec<f32>>> {
        match self.mode {
            BreakEmbed::Bail => bail!("llm endpoint unavailable"),
            BreakEmbed::Cardinality => Ok(vec![vec![0.0], vec![1.0]]),
            BreakEmbed::EmptyVector => Ok(vec![Vec::new()]),
        }
    }
}

fn runtime_over(store: Arc<dyn Store>, client: Arc<dyn ChatStream>) -> Runtime {
    Runtime::new(store, client, MODEL)
}

#[tokio::test]
async fn upsert_embed_failure_is_typed_and_writes_nothing() {
    for mode in [
        BreakEmbed::Bail,
        BreakEmbed::Cardinality,
        BreakEmbed::EmptyVector,
    ] {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let rt = runtime_over(store, Arc::new(BrokenEmbedClient { mode }));
        let err = rt
            .upsert_capability(&payload("doomed capability"), 1_000)
            .await
            .unwrap_err();
        assert!(
            err.downcast_ref::<opencoder_brain::EmbeddingFailed>()
                .is_some(),
            "{mode:?}: {err:#}"
        );
        // Display keeps the historical "embedding failed: …" prefix; the
        // upstream reason is folded into the detail for the Bail class.
        let msg = format!("{err:#}");
        assert!(msg.starts_with("embedding failed"), "{mode:?}: {msg}");
        if matches!(mode, BreakEmbed::Bail) {
            assert!(msg.contains("llm endpoint unavailable"), "{msg}");
        }
        // The embed ran BEFORE any store write: no residual capability row.
        assert!(rt.list_capabilities().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn update_embed_failure_is_typed_and_keeps_old_row() {
    for mode in [
        BreakEmbed::Bail,
        BreakEmbed::Cardinality,
        BreakEmbed::EmptyVector,
    ] {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let seeder = runtime_over(store.clone(), Arc::new(MockChatClient::new()));
        let created = seeder
            .upsert_capability(&payload("original summary"), 1_000)
            .await
            .unwrap();

        let rt = runtime_over(store, Arc::new(BrokenEmbedClient { mode }));
        let err = rt
            .update_capability(
                &created.capability.id,
                &payload("replacement summary"),
                2_000,
            )
            .await
            .unwrap_err();
        assert!(
            err.downcast_ref::<opencoder_brain::EmbeddingFailed>()
                .is_some(),
            "{mode:?}: {err:#}"
        );

        // No partial update: the old row survives with its original content,
        // timestamps and exemplar inputs (the atomic combined write either
        // commits everything or nothing).
        let detail = rt
            .get_capability(&created.capability.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(detail.capability.summary, "original summary");
        assert_eq!(detail.capability.updated_at, 1_000);
        assert_eq!(detail.eng_inputs.len(), 2);
        assert_eq!(rt.list_capabilities().await.unwrap().len(), 1);
    }
}
