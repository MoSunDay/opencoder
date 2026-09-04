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

/// Little-endian f32 bytes — the storage encoding of brain_vectors.emb and the
/// binding format verified to work with `vector32(?)`.
fn le(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

async fn scalar_i64(store: &LibsqlStore, sql: &str) -> i64 {
    let conn = store.conn().await.unwrap();
    let stmt = conn.prepare(sql).await.unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
}

#[tokio::test]
async fn create_get_and_list_roundtrip_with_ordered_eng_inputs() {
    let store = LibsqlStore::open_memory().await.unwrap();

    store
        .create_brain_capability(
            &cap("cap-1", "goal", 100),
            &[
                eng("cap-1", "second", 2),
                eng("cap-1", "first", 1),
                eng("cap-1", "third", 3),
            ],
        )
        .await
        .unwrap();
    store
        .create_brain_capability(&cap("cap-0", "skill", 200), &[eng("cap-0", "only", 0)])
        .await
        .unwrap();

    // get: eng_inputs come back ordered by position with store-assigned ids.
    let detail = store.get_brain_capability("cap-1").await.unwrap().unwrap();
    assert_eq!(detail.capability.summary, "cap-1 summary");
    let contents: Vec<&str> = detail
        .eng_inputs
        .iter()
        .map(|i| i.content.as_str())
        .collect();
    assert_eq!(contents, ["first", "second", "third"]);
    assert!(detail.eng_inputs.iter().all(|i| i.id.is_some()));

    // list: newest first (created_at DESC), inputs attached.
    let all = store.list_brain_capabilities().await.unwrap();
    assert_eq!(
        all.iter()
            .map(|d| d.capability.id.as_str())
            .collect::<Vec<_>>(),
        ["cap-0", "cap-1"]
    );
    assert_eq!(all[1].eng_inputs.len(), 3);

    assert!(store
        .get_brain_capability("missing")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn update_replaces_eng_inputs() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store
        .create_brain_capability(
            &cap("cap-1", "goal", 100),
            &[eng("cap-1", "old-a", 1), eng("cap-1", "old-b", 2)],
        )
        .await
        .unwrap();

    let mut updated = cap("cap-1", "goal", 100);
    updated.summary = "rewritten".into();
    updated.updated_at = 300;
    store
        .update_brain_capability(&updated, &[eng("cap-1", "new-only", 1)])
        .await
        .unwrap();

    let detail = store.get_brain_capability("cap-1").await.unwrap().unwrap();
    assert_eq!(detail.capability.summary, "rewritten");
    assert_eq!(detail.capability.updated_at, 300);
    assert_eq!(detail.eng_inputs.len(), 1);
    assert_eq!(detail.eng_inputs[0].content, "new-only");
    assert_eq!(
        scalar_i64(&store, "SELECT COUNT(*) FROM brain_eng_inputs").await,
        1,
        "old eng_input rows must be gone after update"
    );
}

#[tokio::test]
async fn delete_cascades_eng_inputs_and_vectors() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store
        .create_brain_capability(&cap("cap-1", "goal", 100), &[eng("cap-1", "a", 1)])
        .await
        .unwrap();
    store
        .upsert_brain_vector("cap-1", 4, "emb", &le(&[1.0, 0.0, 0.0, 0.0]), 1)
        .await
        .unwrap();

    store.delete_brain_capability("cap-1").await.unwrap();

    assert!(store.get_brain_capability("cap-1").await.unwrap().is_none());
    assert_eq!(
        scalar_i64(&store, "SELECT COUNT(*) FROM brain_eng_inputs").await,
        0
    );
    assert_eq!(
        scalar_i64(&store, "SELECT COUNT(*) FROM brain_vectors").await,
        0
    );
    assert!(store
        .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn upsert_vector_is_idempotent() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store
        .create_brain_capability(&cap("cap-1", "goal", 100), &[])
        .await
        .unwrap();

    store
        .upsert_brain_vector("cap-1", 4, "emb", &le(&[1.0, 0.0, 0.0, 0.0]), 1)
        .await
        .unwrap();
    store
        .upsert_brain_vector("cap-1", 4, "emb", &le(&[0.0, 1.0, 0.0, 0.0]), 2)
        .await
        .unwrap();

    assert_eq!(
        scalar_i64(
            &store,
            "SELECT COUNT(*) FROM brain_vectors WHERE capability_id='cap-1'"
        )
        .await,
        1,
        "second upsert must replace, not duplicate"
    );
    // The latest embedding won: query [0,1,0,0] is now at distance 0.
    let hits = store
        .search_brain_vectors("emb", &le(&[0.0, 1.0, 0.0, 0.0]), 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].distance.abs() < 1e-6);
}

#[tokio::test]
async fn vector_distance_cos_orders_hits_and_reports_cosine_distance() {
    let store = LibsqlStore::open_memory().await.unwrap();
    // e1 aligned with the query, e2 orthogonal, e3 opposite.
    let vectors: &[(&str, [f32; 4])] = &[
        ("cap-e1", [1.0, 0.0, 0.0, 0.0]),
        ("cap-e2", [0.0, 1.0, 0.0, 0.0]),
        ("cap-e3", [-1.0, 0.0, 0.0, 0.0]),
    ];
    for (idx, (id, v)) in vectors.iter().enumerate() {
        store
            .create_brain_capability(&cap(id, "goal", 100 + idx as i64), &[])
            .await
            .unwrap();
        store
            .upsert_brain_vector(id, 4, "emb", &le(v), 1)
            .await
            .unwrap();
    }

    let hits = store
        .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 10)
        .await
        .unwrap();

    assert_eq!(hits.len(), 3, "all three vectors must be reachable");
    assert_eq!(
        hits[0].capability.id, "cap-e1",
        "aligned vector ranks first"
    );
    assert!(hits[0].distance.abs() < 1e-6, "identical direction → d ≈ 0");
    assert!(
        hits.windows(2).all(|w| w[0].distance <= w[1].distance),
        "distances must be non-decreasing (vector_distance_cos ORDER BY)"
    );
    let by_id = |id: &str| {
        hits.iter()
            .find(|h| h.capability.id == id)
            .unwrap()
            .distance
    };
    assert!((by_id("cap-e2") - 1.0).abs() < 1e-6, "orthogonal → d ≈ 1");
    assert!((by_id("cap-e3") - 2.0).abs() < 1e-6, "opposite → d ≈ 2");

    // LIMIT is honored.
    let top1 = store
        .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 1)
        .await
        .unwrap();
    assert_eq!(top1.len(), 1);
    assert_eq!(top1[0].capability.id, "cap-e1");
}

#[tokio::test]
async fn search_filters_by_embedding_model() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store
        .create_brain_capability(&cap("cap-a", "goal", 100), &[])
        .await
        .unwrap();
    store
        .create_brain_capability(&cap("cap-b", "goal", 200), &[])
        .await
        .unwrap();
    store
        .upsert_brain_vector("cap-a", 4, "emb-a", &le(&[1.0, 0.0, 0.0, 0.0]), 1)
        .await
        .unwrap();
    store
        .upsert_brain_vector("cap-b", 4, "emb-b", &le(&[1.0, 0.0, 0.0, 0.0]), 1)
        .await
        .unwrap();

    let only_a = store
        .search_brain_vectors("emb-a", &le(&[1.0, 0.0, 0.0, 0.0]), 10)
        .await
        .unwrap();
    assert_eq!(only_a.len(), 1);
    assert_eq!(only_a[0].capability.id, "cap-a");

    let only_b = store
        .search_brain_vectors("emb-b", &le(&[1.0, 0.0, 0.0, 0.0]), 10)
        .await
        .unwrap();
    assert_eq!(only_b.len(), 1);
    assert_eq!(only_b[0].capability.id, "cap-b");

    let none = store
        .search_brain_vectors("emb-c", &le(&[1.0, 0.0, 0.0, 0.0]), 10)
        .await
        .unwrap();
    assert!(none.is_empty(), "unknown model must match nothing");
}

/// Precomputed embedding payload handed to the combined store methods —
/// mirrors what the brain runtime does (embed first, persist second).
fn vec_write(dim: i64, model: &str, v: &[f32], embedded_at: i64) -> BrainVectorWrite {
    BrainVectorWrite {
        dim,
        model: model.into(),
        emb: le(v),
        embedded_at,
    }
}

#[tokio::test]
async fn create_with_vector_persists_capability_inputs_and_vector_together() {
    let store = LibsqlStore::open_memory().await.unwrap();

    store
        .create_brain_capability_with_vector(
            &cap("cap-1", "goal", 100),
            &[
                eng("cap-1", "second", 2),
                eng("cap-1", "first", 1),
                eng("cap-1", "third", 3),
            ],
            &vec_write(4, "emb", &[1.0, 0.0, 0.0, 0.0], 150),
        )
        .await
        .unwrap();

    // Capability and ordered exemplar inputs landed.
    let detail = store.get_brain_capability("cap-1").await.unwrap().unwrap();
    assert_eq!(detail.capability.summary, "cap-1 summary");
    let contents: Vec<&str> = detail
        .eng_inputs
        .iter()
        .map(|i| i.content.as_str())
        .collect();
    assert_eq!(contents, ["first", "second", "third"]);

    // The vector landed in the same call and is searchable.
    let hits = store
        .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 5)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].capability.id, "cap-1");
    assert!(hits[0].distance.abs() < 1e-6);
    assert_eq!(
        scalar_i64(&store, "SELECT COUNT(*) FROM brain_vectors").await,
        1,
        "combined create must leave exactly one vector row"
    );
}

#[tokio::test]
async fn update_with_vector_replaces_content_inputs_and_vector() {
    let store = LibsqlStore::open_memory().await.unwrap();

    store
        .create_brain_capability_with_vector(
            &cap("cap-1", "goal", 100),
            &[eng("cap-1", "old-a", 1), eng("cap-1", "old-b", 2)],
            &vec_write(4, "emb-a", &[1.0, 0.0, 0.0, 0.0], 150),
        )
        .await
        .unwrap();

    // Combined update: new content, new eng_inputs and a DIFFERENT embedding
    // (other model, other bytes) must all replace the old state together.
    let mut updated = cap("cap-1", "goal", 100);
    updated.summary = "rewritten".into();
    updated.updated_at = 300;
    store
        .update_brain_capability_with_vector(
            &updated,
            &[eng("cap-1", "new-only", 1)],
            &vec_write(4, "emb-b", &[0.0, 1.0, 0.0, 0.0], 350),
        )
        .await
        .unwrap();

    // Content and eng_inputs were replaced.
    let detail = store.get_brain_capability("cap-1").await.unwrap().unwrap();
    assert_eq!(detail.capability.summary, "rewritten");
    assert_eq!(detail.capability.updated_at, 300);
    assert_eq!(detail.eng_inputs.len(), 1);
    assert_eq!(detail.eng_inputs[0].content, "new-only");

    // The vector was REPLACED, not duplicated: the new model answers search
    // with the new bytes at distance 0 …
    let hits = store
        .search_brain_vectors("emb-b", &le(&[0.0, 1.0, 0.0, 0.0]), 5)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].capability.id, "cap-1");
    assert!(hits[0].distance.abs() < 1e-6, "new embedding must be live");
    // … while the old model no longer matches anything.
    assert!(
        store
            .search_brain_vectors("emb-a", &le(&[1.0, 0.0, 0.0, 0.0]), 5)
            .await
            .unwrap()
            .is_empty(),
        "old-model vector must be gone, not left behind"
    );
    assert_eq!(
        scalar_i64(&store, "SELECT COUNT(*) FROM brain_vectors").await,
        1,
        "combined update must replace the vector row, not add a second one"
    );
}

/// v14 → v15: a hand-built v14 database gains the three brain tables on
/// reopen (bootstrap → migrate), and the brain API round-trips through the
/// migrated schema. Mirrors the hand-written-old-schema pattern in
/// store_migrations.rs.
#[tokio::test]
async fn migration_v14_to_v15_creates_brain_tables() {
    let dir: TempDir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("brain-migrate.db");
    {
        let db = libsql::Builder::new_local(&db_path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (14)", ())
            .await
            .unwrap();
    }

    let store = LibsqlStore::open(&db_path).await.unwrap();

    // Schema version bumped to the latest (17).
    assert_eq!(
        scalar_i64(&store, "SELECT version FROM schema_version LIMIT 1").await,
        17
    );

    // All three brain tables now exist.
    for table in ["brain_capabilities", "brain_eng_inputs", "brain_vectors"] {
        assert_eq!(
            scalar_i64(
                &store,
                &format!(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{table}'"
                )
            )
            .await,
            1,
            "{table} must exist after v14→v15 migration"
        );
    }

    // The migrated schema is immediately usable end-to-end.
    store
        .create_brain_capability(&cap("cap-m", "goal", 1), &[eng("cap-m", "in", 0)])
        .await
        .unwrap();
    store
        .upsert_brain_vector("cap-m", 4, "emb", &le(&[1.0, 0.0, 0.0, 0.0]), 1)
        .await
        .unwrap();
    let hits = store
        .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 5)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].capability.id, "cap-m");
}
