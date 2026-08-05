//! Round-trip for the v7 `summary_images_json` column: writing image URLs via
//! `SessionPatch::summary_images` and reading them back via `get_session`.
//! Also covers the NULL -> empty-vec backward-compat path (old sessions).

use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store};
use tempfile::TempDir;

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    store
        .create_session(&SessionMeta {
            id: "s1".into(),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    (dir, store)
}

#[tokio::test]
async fn summary_images_round_trip() {
    let (_dir, store) = fresh().await;

    // A freshly created session has no persisted summary images -> empty vec.
    let m0 = store.get_session("s1").await.unwrap().unwrap();
    assert!(m0.summary_images.is_empty(), "fresh session has no summary images");

    // Persist a set of image URLs.
    store
        .update_session(
            "s1",
            &SessionPatch {
                summary_images: Some(vec!["u1".into(), "u2".into()]),
                updated_at: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let m1 = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(m1.summary_images, vec!["u1".to_string(), "u2".to_string()]);

    // Overwrite with a different (shorter) set.
    store
        .update_session(
            "s1",
            &SessionPatch {
                summary_images: Some(vec!["only.png".into()]),
                updated_at: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let m2 = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(m2.summary_images, vec!["only.png".to_string()]);

    // Emptying the list persists an empty array (not NULL).
    store
        .update_session(
            "s1",
            &SessionPatch {
                summary_images: Some(Vec::new()),
                updated_at: Some(3),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let m3 = store.get_session("s1").await.unwrap().unwrap();
    assert!(m3.summary_images.is_empty(), "empty list persists as empty");
}

/// A session created before the column existed (or with summary_images never
/// set) reads back as an empty vec -- the backward-compatible default.
#[tokio::test]
async fn summary_images_null_reads_as_empty() {
    let (_dir, store) = fresh().await;
    // Update a DIFFERENT field so the row is touched but summary_images_json
    // stays NULL (the patch omits summary_images entirely).
    store
        .update_session(
            "s1",
            &SessionPatch {
                summary: Some("text".into()),
                updated_at: Some(9),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let m = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(m.summary, Some("text".into()));
    assert!(m.summary_images.is_empty(), "NULL column reads back as empty vec");
}
