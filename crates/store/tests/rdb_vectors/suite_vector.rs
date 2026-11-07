//! Vector-row lifecycle and nearest-neighbour search: atomic combined
//! create/update, idempotent upserts, cosine ranking, model scoping and
//! the capability foreign key. Strict payload validation and corrupt-row
//! handling live in `suite_vector_validation.rs`.

use super::*;

#[tokio::test]
async fn create_with_vector_persists_capability_inputs_and_vector_together() {
    let store = mem().await;
    store
        .create_brain_capability_with_vector(
            &cap("cap-1", "goal", 100),
            &[
                eng("cap-1", "first", 1),
                eng("cap-1", "second", 2),
                eng("cap-1", "third", 3),
            ],
            &vec_write(4, "emb", &[1.0, 0.0, 0.0, 0.0], 150),
        )
        .await
        .unwrap();

    let detail = store.get_brain_capability("cap-1").await.unwrap().unwrap();
    let contents: Vec<&str> = detail
        .eng_inputs
        .iter()
        .map(|i| i.content.as_str())
        .collect();
    assert_eq!(contents, ["first", "second", "third"]);

    // The vector landed in the same call and is searchable — exactly one
    // embedding row exists for the capability.
    let hits = store
        .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 5)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].capability.id, "cap-1");
    assert!(hits[0].distance.abs() < 1e-6);
}

#[tokio::test]
async fn update_with_vector_replaces_content_inputs_and_vector() {
    let store = mem().await;
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
}

#[tokio::test]
async fn upsert_vector_is_idempotent_and_requires_the_capability() {
    let store = mem().await;
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

    // The latest embedding won and the row was replaced, not duplicated:
    // one hit, and a query of the FIRST embedding is now orthogonal
    // (distance 1) instead of identical (distance 0).
    let hits = store
        .search_brain_vectors("emb", &le(&[0.0, 1.0, 0.0, 0.0]), 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "second upsert must replace, not duplicate");
    assert!(hits[0].distance.abs() < 1e-6);
    let stale = store
        .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 10)
        .await
        .unwrap();
    assert_eq!(stale.len(), 1);
    assert!((stale[0].distance - 1.0).abs() < 1e-6);

    // The embedding row carries the capability foreign key: an upsert for a
    // missing capability cannot strand an orphan vector.
    assert!(
        store
            .upsert_brain_vector("ghost", 4, "emb", &le(&[1.0, 0.0, 0.0, 0.0]), 3)
            .await
            .is_err(),
        "upsert must reject a vector without its capability row"
    );
    // The rejected write left the surviving state untouched.
    assert_eq!(
        store
            .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn vector_distance_orders_hits_and_reports_cosine_distance() {
    let store = mem().await;
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
    assert_eq!(hits.len(), 3);
    assert!(
        hits.windows(2).all(|w| w[0].distance <= w[1].distance),
        "distances must be non-decreasing (cosine-distance ORDER BY)"
    );
    let by_id = |id: &str| {
        hits.iter()
            .find(|h| h.capability.id == id)
            .unwrap()
            .distance
    };
    assert!((by_id("cap-e1") - 0.0).abs() < 1e-6, "aligned → d ≈ 0");
    assert!((by_id("cap-e2") - 1.0).abs() < 1e-6, "orthogonal → d ≈ 1");
    assert!((by_id("cap-e3") - 2.0).abs() < 1e-6, "opposite → d ≈ 2");

    // LIMIT is honored, best hit first.
    let top1 = store
        .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 1)
        .await
        .unwrap();
    assert_eq!(top1.len(), 1);
    assert_eq!(top1[0].capability.id, "cap-e1");
}

#[tokio::test]
async fn search_filters_by_embedding_model() {
    let store = mem().await;
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
