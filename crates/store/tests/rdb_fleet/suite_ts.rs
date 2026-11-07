//! TsRegistry over the rdb engine: row round-trips and the migration marker.
use opencoder_store::ts_registry::{TsRecord, TsRegistry};
use std::path::{Path, PathBuf};

fn record(id: &str, workdir: Option<&str>, updated_at: i64) -> TsRecord {
    TsRecord {
        id: id.into(),
        workdir: workdir.map(PathBuf::from),
        store_dir: Some(PathBuf::from("/store")),
        created_at: 1,
        updated_at,
        title: Some(format!("title-{id}")),
        preview: format!("preview-{id}"),
    }
}

#[tokio::test]
async fn open_maps_a_file_path_to_an_rdb_directory_and_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ts.db");
    {
        let registry = TsRegistry::open(&path).await.unwrap();
        registry.upsert(&record("a", Some("/a"), 2)).await.unwrap();
        registry.mark_migrated().await.unwrap();
    }
    assert!(!path.exists(), "the legacy file path is never created");
    assert!(dir.path().join("ts.db.rdb").is_dir());

    let reopened = TsRegistry::open(&path).await.unwrap();
    assert!(reopened.is_migrated().await.unwrap());
    assert_eq!(reopened.list().await.unwrap().len(), 1);
    assert_eq!(
        reopened.get("a").await.unwrap().unwrap().workdir.as_deref(),
        Some(Path::new("/a"))
    );
}

#[tokio::test]
async fn upsert_list_and_get_round_trip_ordered_by_id() {
    let registry = TsRegistry::open_memory().await.unwrap();
    assert!(registry.list().await.unwrap().is_empty());
    assert!(!registry.is_migrated().await.unwrap(), "fresh registry");

    registry.upsert(&record("b", None, 5)).await.unwrap();
    registry.upsert(&record("a", Some("/a"), 9)).await.unwrap();
    let listed = registry.list().await.unwrap();
    assert_eq!(
        listed.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["a", "b"],
        "rows are iterated by id, not by updated_at"
    );
    assert_eq!(listed[1].workdir, None, "optional workdir round-trips");
    assert_eq!(listed[1].preview, "preview-b");

    let got = registry.get("a").await.unwrap().unwrap();
    assert_eq!(got.workdir.as_deref(), Some(Path::new("/a")));
    assert_eq!(got.store_dir.as_deref(), Some(Path::new("/store")));
    assert_eq!(got.updated_at, 9);
    assert_eq!(got.title.as_deref(), Some("title-a"));
    assert!(registry.get("missing").await.unwrap().is_none());
}

#[tokio::test]
async fn upsert_replaces_an_existing_row_without_duplicating_it() {
    let registry = TsRegistry::open_memory().await.unwrap();
    registry
        .upsert(&record("a", Some("/old"), 1))
        .await
        .unwrap();
    let mut updated = record("a", Some("/new"), 7);
    updated.title = Some("updated".into());
    registry.upsert(&updated).await.unwrap();

    let listed = registry.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].workdir.as_deref(), Some(Path::new("/new")));
    assert_eq!(listed[0].title.as_deref(), Some("updated"));
    assert_eq!(listed[0].updated_at, 7);
}

#[tokio::test]
async fn delete_removes_only_the_target_and_is_idempotent() {
    let registry = TsRegistry::open_memory().await.unwrap();
    registry.upsert(&record("a", Some("/a"), 1)).await.unwrap();
    registry.upsert(&record("b", Some("/b"), 2)).await.unwrap();

    registry.delete("a").await.unwrap();
    assert!(registry.get("a").await.unwrap().is_none());
    let listed = registry.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "b");

    registry.delete("a").await.unwrap();
    assert_eq!(registry.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn migrated_marker_is_idempotent_and_shared_across_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ts.db");
    let registry = TsRegistry::open(&path).await.unwrap();
    assert!(!registry.is_migrated().await.unwrap());
    registry.mark_migrated().await.unwrap();
    assert!(registry.is_migrated().await.unwrap());
    registry.mark_migrated().await.unwrap();
    assert!(registry.is_migrated().await.unwrap(), "mark is idempotent");
    drop(registry);

    let reopened = TsRegistry::open(&path).await.unwrap();
    assert!(reopened.is_migrated().await.unwrap());
    assert!(reopened.list().await.unwrap().is_empty());

    let other = TsRegistry::open_memory().await.unwrap();
    assert!(
        !other.is_migrated().await.unwrap(),
        "the marker is per registry"
    );
}
