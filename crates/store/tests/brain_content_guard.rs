use opencoder_store::{
    BrainCapabilityRecord, BrainContentConflict, BrainEngInputRecord, BrainVectorWrite,
    LibsqlStore, Store,
};

#[tokio::test]
async fn stale_content_cannot_replace_examples_or_vector_even_with_same_timestamp() {
    let store = LibsqlStore::open_memory().await.unwrap();
    let mut record = BrainCapabilityRecord {
        id: "cap".into(),
        capability_type: "tool".into(),
        summary: "first".into(),
        input_desc: "input".into(),
        output_desc: "output".into(),
        created_at: 1,
        updated_at: 1,
    };
    let vector = BrainVectorWrite {
        dim: 2,
        model: "model".into(),
        emb: [1.0_f32, 0.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect(),
        embedded_at: 1,
    };
    let examples = [BrainEngInputRecord {
        id: None,
        capability_id: "cap".into(),
        content: "original".into(),
        position: 0,
    }];
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
    record.summary = "stale overwrite".into();
    let replacement = BrainVectorWrite {
        emb: [0.0_f32, 1.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect(),
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
    assert!(err.downcast_ref::<BrainContentConflict>().is_some());
    let kept = store.get_brain_capability("cap").await.unwrap().unwrap();
    assert_eq!(kept.content_version(), concurrent.content_version());
    assert!(
        store
            .search_brain_vectors("model", &vector.emb, 1)
            .await
            .unwrap()[0]
            .distance
            .abs()
            < 1e-6
    );
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
    assert!(err.downcast_ref::<BrainContentConflict>().is_some());
    assert!(store.get_brain_capability("cap").await.unwrap().is_none());
}
