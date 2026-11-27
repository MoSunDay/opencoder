use opencoder_store::{
    export_bundle, import_bundle, read_bundle, write_bundle, SessionMeta, SessionModelOverrides,
    Store, TursoStore,
};

fn meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some("model metadata".into()),
        agent: Some("act".into()),
        model: Some("shared/model".into()),
        created_at: 1,
        updated_at: 1,
        ..Default::default()
    }
}

fn overrides(act: Option<&str>, plan: Option<&str>) -> SessionModelOverrides {
    SessionModelOverrides {
        act: act.map(str::to_owned),
        plan: plan.map(str::to_owned),
        shared: None,
    }
}

#[tokio::test]
async fn legacy_and_missing_rows_read_as_legacy_null() {
    let store = TursoStore::open_memory().await.unwrap();
    store.create_session(&meta("legacy")).await.unwrap();
    assert_eq!(
        store.get_session_model_overrides("legacy").await.unwrap(),
        None
    );
    assert_eq!(
        store.get_session_model_overrides("missing").await.unwrap(),
        None
    );
}

#[tokio::test]
async fn fresh_rows_distinguish_empty_inheritance_from_legacy_null() {
    let store = TursoStore::open_memory().await.unwrap();
    store
        .create_session_with_model_overrides(
            &meta("fresh"),
            Some(&SessionModelOverrides::default()),
        )
        .await
        .unwrap();
    assert_eq!(
        store.get_session_model_overrides("fresh").await.unwrap(),
        Some(SessionModelOverrides::default())
    );

    let mut selected = overrides(Some("provider/act-a"), Some("provider/plan-a"));
    selected.shared = Some("provider/shared-a".into());
    store
        .set_session_model_overrides("fresh", Some(&selected))
        .await
        .unwrap();
    assert_eq!(
        store.get_session_model_overrides("fresh").await.unwrap(),
        Some(selected)
    );
    assert_eq!(
        store
            .get_session("fresh")
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("shared/model")
    );

    store
        .set_session_model_overrides("fresh", None)
        .await
        .unwrap();
    assert_eq!(
        store.get_session_model_overrides("fresh").await.unwrap(),
        None
    );
    store
        .set_session_model_overrides("missing", Some(&SessionModelOverrides::default()))
        .await
        .unwrap_err();
}

#[test]
fn old_empty_and_mode_only_json_have_no_shared_session_selection() {
    let empty: SessionModelOverrides = serde_json::from_str("{}").unwrap();
    assert_eq!(empty, SessionModelOverrides::default());
    assert_eq!(empty.shared, None);
    assert_eq!(serde_json::to_value(&empty).unwrap(), serde_json::json!({}));

    let old: SessionModelOverrides =
        serde_json::from_str(r#"{"act":"provider/act","plan":"provider/plan"}"#).unwrap();
    assert_eq!(old.shared, None);
    assert_eq!(old.act.as_deref(), Some("provider/act"));
    assert_eq!(old.plan.as_deref(), Some("provider/plan"));

    let selected = SessionModelOverrides {
        shared: Some("provider/session-shared".into()),
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_value(&selected).unwrap(),
        serde_json::json!({"shared": "provider/session-shared"})
    );
}

#[tokio::test]
async fn bundle_without_metadata_field_remains_legacy_and_imports_as_null() {
    let source = TursoStore::open_memory().await.unwrap();
    source.create_session(&meta("old-bundle")).await.unwrap();
    let exported = export_bundle(&source, "old-bundle").await.unwrap();
    let mut value = serde_json::to_value(&exported).unwrap();
    value.as_object_mut().unwrap().remove("model_overrides");
    let payload = serde_json::to_vec(&value).unwrap();
    let mut bytes = b"OPENCODR".to_vec();
    bytes.extend(1_u32.to_le_bytes());
    bytes.extend((payload.len() as u64).to_le_bytes());
    bytes.extend(payload);

    let decoded = read_bundle(&mut bytes.as_slice()).unwrap();
    assert_eq!(decoded.model_overrides, None);
    let target = TursoStore::open_memory().await.unwrap();
    import_bundle(&target, &decoded, None).await.unwrap();
    assert_eq!(
        target
            .get_session_model_overrides("old-bundle")
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn bundle_export_import_preserves_mode_metadata() {
    let source = TursoStore::open_memory().await.unwrap();
    let mut selected = overrides(Some("provider/act-a"), None);
    selected.shared = Some("provider/shared-bundle".into());
    source
        .create_session_with_model_overrides(&meta("source"), Some(&selected))
        .await
        .unwrap();

    let exported = export_bundle(&source, "source").await.unwrap();
    assert_eq!(exported.model_overrides, Some(selected.clone()));
    let mut bytes = Vec::new();
    write_bundle(&exported, &mut bytes).unwrap();
    let decoded = read_bundle(&mut bytes.as_slice()).unwrap();
    assert_eq!(decoded.model_overrides, Some(selected.clone()));

    let target = TursoStore::open_memory().await.unwrap();
    import_bundle(&target, &decoded, None).await.unwrap();
    assert_eq!(
        target.get_session_model_overrides("source").await.unwrap(),
        Some(selected)
    );
}

#[tokio::test]
async fn v41_rows_gain_null_metadata_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v41.db");
    {
        let store = TursoStore::open(&path).await.unwrap();
        store.create_session(&meta("old")).await.unwrap();
        let conn = store.conn().await.unwrap();
        conn.execute("ALTER TABLE sessions DROP COLUMN model_overrides_json", ())
            .await
            .unwrap();
        conn.execute("UPDATE schema_version SET version = 41", ())
            .await
            .unwrap();
    }

    let store = TursoStore::open(&path).await.unwrap();
    assert_eq!(
        store.get_session_model_overrides("old").await.unwrap(),
        None
    );
    let selected = overrides(Some("provider/act-b"), None);
    store
        .set_session_model_overrides("old", Some(&selected))
        .await
        .unwrap();
    assert_eq!(
        store.get_session_model_overrides("old").await.unwrap(),
        Some(selected)
    );
}
