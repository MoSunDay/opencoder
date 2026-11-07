//! Capability CRUD, exemplar-input ordering/replacement, cascade delete,
//! duplicate-id rollback and guarded version updates.

use super::*;

#[tokio::test]
async fn create_get_and_list_roundtrip_with_ordered_eng_inputs() {
    let store = mem().await;

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
    let store = mem().await;
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
    let contents: Vec<&str> = detail
        .eng_inputs
        .iter()
        .map(|i| i.content.as_str())
        .collect();
    assert_eq!(
        contents,
        ["new-only"],
        "replace semantics drop removed inputs"
    );

    // Equal positions keep insertion (row-id) order.
    store
        .update_brain_capability(
            &updated,
            &[
                eng("cap-1", "z-last-written", 1),
                eng("cap-1", "a-first-written", 1),
            ],
        )
        .await
        .unwrap();
    let detail = store.get_brain_capability("cap-1").await.unwrap().unwrap();
    let contents: Vec<&str> = detail
        .eng_inputs
        .iter()
        .map(|i| i.content.as_str())
        .collect();
    assert_eq!(contents, ["z-last-written", "a-first-written"]);
}

#[tokio::test]
async fn delete_cascades_eng_inputs_and_vectors() {
    let store = mem().await;
    store
        .create_brain_capability_with_vector(
            &cap("cap-1", "goal", 100),
            &[eng("cap-1", "in", 0)],
            &vec_write(4, "emb", &[1.0, 0.0, 0.0, 0.0], 150),
        )
        .await
        .unwrap();

    store.delete_brain_capability("cap-1").await.unwrap();

    assert!(store.get_brain_capability("cap-1").await.unwrap().is_none());
    assert!(
        store
            .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 10)
            .await
            .unwrap()
            .is_empty(),
        "vector row must cascade away with the capability"
    );
    // Deleting again is a no-op, and re-creating the id is allowed again.
    store.delete_brain_capability("cap-1").await.unwrap();
    store
        .create_brain_capability(&cap("cap-1", "goal", 200), &[])
        .await
        .unwrap();
    assert!(store.get_brain_capability("cap-1").await.unwrap().is_some());
}

#[tokio::test]
async fn duplicate_capability_id_rejects_and_rolls_back_the_vector() {
    let store = mem().await;
    store
        .create_brain_capability_with_vector(
            &cap("cap-1", "goal", 100),
            &[eng("cap-1", "in", 0)],
            &vec_write(4, "emb-a", &[1.0, 0.0, 0.0, 0.0], 150),
        )
        .await
        .unwrap();

    let err = store
        .create_brain_capability_with_vector(
            &cap("cap-1", "goal", 999),
            &[eng("cap-1", "intruder", 0)],
            &vec_write(4, "emb-b", &[0.0, 1.0, 0.0, 0.0], 950),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("UNIQUE constraint failed: brain_capabilities.id"),
        "duplicate id must surface the SQL conflict text, got: {err}"
    );

    // The failed batch committed nothing: original content and vector stay.
    let detail = store.get_brain_capability("cap-1").await.unwrap().unwrap();
    assert_eq!(detail.capability.created_at, 100);
    assert_eq!(detail.eng_inputs.len(), 1);
    assert_eq!(detail.eng_inputs[0].content, "in");
    assert!(
        store
            .search_brain_vectors("emb-b", &le(&[0.0, 1.0, 0.0, 0.0]), 5)
            .await
            .unwrap()
            .is_empty(),
        "no capability without its vector — and no vector without its capability"
    );
    let hits = store
        .search_brain_vectors("emb-a", &le(&[1.0, 0.0, 0.0, 0.0]), 5)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].distance.abs() < 1e-6);

    // The plain create path carries the same guard.
    assert!(store
        .create_brain_capability(&cap("cap-1", "goal", 100), &[])
        .await
        .is_err());
}

#[tokio::test]
async fn stale_content_cannot_replace_examples_or_vector_even_with_same_timestamp() {
    let store = mem().await;
    let mut record = BrainCapabilityRecord {
        id: "cap".into(),
        capability_type: "tool".into(),
        summary: "first".into(),
        input_desc: "input".into(),
        output_desc: "output".into(),
        created_at: 1,
        updated_at: 1,
    };
    let vector = vec_write(2, "model", &[1.0, 0.0], 1);
    let examples = [eng("cap", "original", 0)];
    store
        .create_brain_capability_with_vector(&record, &examples, &vector)
        .await
        .unwrap();
    let original = store.get_brain_capability("cap").await.unwrap().unwrap();
    record.summary = "concurrent edit".into();
    store
        .update_brain_capability_if_version(
            &record,
            &examples,
            &vector,
            Some(&original.content_version()),
        )
        .await
        .unwrap();
    let concurrent = store.get_brain_capability("cap").await.unwrap().unwrap();
    assert_ne!(concurrent.content_version(), original.content_version());

    // A writer holding the stale version is rejected with the typed conflict.
    record.summary = "stale overwrite".into();
    let replacement = BrainVectorWrite {
        emb: le(&[0.0, 1.0]),
        ..vector.clone()
    };
    let err = store
        .update_brain_capability_if_version(
            &record,
            &[],
            &replacement,
            Some(&original.content_version()),
        )
        .await
        .unwrap_err();
    assert!(err
        .downcast_ref::<opencoder_store::BrainContentConflict>()
        .is_some());

    // Content, examples and the embedding all kept the winning writer's state.
    let kept = store.get_brain_capability("cap").await.unwrap().unwrap();
    assert_eq!(kept.content_version(), concurrent.content_version());
    let hits = store
        .search_brain_vectors("model", &vector.emb, 1)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].distance.abs() < 1e-6);

    // A guarded update of a deleted row is a conflict, never a resurrect.
    store.delete_brain_capability("cap").await.unwrap();
    let err = store
        .update_brain_capability_if_version(
            &record,
            &examples,
            &vector,
            Some(&concurrent.content_version()),
        )
        .await
        .unwrap_err();
    assert!(err
        .downcast_ref::<opencoder_store::BrainContentConflict>()
        .is_some());
    assert!(store.get_brain_capability("cap").await.unwrap().is_none());
}
