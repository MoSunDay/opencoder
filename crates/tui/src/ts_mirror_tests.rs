//! Tests for `TsMirrorStore` (see `ts_mirror.rs`). Mirrors the `#[path]`
//! convention used by `app_loop_bugfix_tests.rs` / `composer_delete_tests.rs`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use opencoder_core::Message;
use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store, TsRegistry};
use tokio::sync::Mutex;

use super::{maybe_wrap_at, TsMirrorStore};

fn ts_meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.to_string(),
        title: Some("seed title".into()),
        created_at: 100,
        updated_at: 100,
        ..Default::default()
    }
}

async fn fixture() -> (TsMirrorStore, tempfile::TempDir, PathBuf) {
    let inner: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path().join("project");
    tokio::fs::create_dir_all(&workdir).await.unwrap();
    let registry = TsRegistry::open(tmp.path().join("ts.db")).await.unwrap();
    let mirror = TsMirrorStore {
        inner,
        registry,
        workdir: workdir.clone(),
        known: Mutex::new(HashSet::new()),
    };
    (mirror, tmp, workdir)
}

#[tokio::test]
async fn ts_session_is_registered_with_title_and_store_dir() {
    let (mirror, _tmp, workdir) = fixture().await;
    mirror.create_session(&ts_meta("01AAA")).await.unwrap();

    let record = mirror
        .registry
        .get("01AAA")
        .await
        .unwrap()
        .expect("registered");
    assert_eq!(record.title.as_deref(), Some("seed title"));
    assert_eq!(record.workdir.as_deref(), Some(workdir.as_path()));
    assert_eq!(
        record.store_dir.as_deref(),
        Some(opencoder_core::data_dir_for(&workdir).as_path())
    );
    assert_eq!(record.created_at, 100);
    assert!(mirror.known.lock().await.contains("01AAA"));
}

#[tokio::test]
async fn plain_session_is_not_registered() {
    let (mirror, _tmp, _workdir) = fixture().await;
    let mut meta = ts_meta("02BBB");
    meta.model = Some("provider/model".into());
    mirror.create_session(&meta).await.unwrap();

    assert!(mirror.registry.get("02BBB").await.unwrap().is_none());
    assert!(mirror.registry.list().await.unwrap().is_empty());
    assert!(!mirror.known.lock().await.contains("02BBB"));
}

#[tokio::test]
async fn first_user_message_writes_preview_once() {
    let (mirror, _tmp, _workdir) = fixture().await;
    mirror.create_session(&ts_meta("01AAA")).await.unwrap();

    mirror
        .append_message("01AAA", &Message::assistant("m1"))
        .await
        .unwrap();
    assert_eq!(
        mirror.registry.get("01AAA").await.unwrap().unwrap().preview,
        "",
        "assistant messages never fill the preview"
    );

    let long = format!("hello {}", "x".repeat(200));
    mirror
        .append_message("01AAA", &Message::user("m2", long.clone()))
        .await
        .unwrap();
    let preview = mirror.registry.get("01AAA").await.unwrap().unwrap().preview;
    assert_eq!(preview.chars().count(), 80, "preview capped at 80 chars");
    assert!(long.starts_with(&preview));

    mirror
        .append_message("01AAA", &Message::user("m3", "second message"))
        .await
        .unwrap();
    assert_eq!(
        mirror.registry.get("01AAA").await.unwrap().unwrap().preview,
        preview,
        "preview is write-once"
    );
}

#[tokio::test]
async fn unknown_session_never_mirrors() {
    let (mirror, _tmp, _workdir) = fixture().await;
    // Session exists in the inner store but was never created through this
    // mirror instance (e.g. resumed plain session) — appends must not write.
    let mut meta = ts_meta("GHOST");
    meta.model = Some("m".into());
    mirror.inner.create_session(&meta).await.unwrap();
    mirror
        .append_message("GHOST", &Message::user("m1", "hi"))
        .await
        .unwrap();
    assert!(mirror.registry.get("GHOST").await.unwrap().is_none());
}

#[tokio::test]
async fn title_patch_mirrors_existing_row_only() {
    let (mirror, _tmp, _workdir) = fixture().await;
    mirror.create_session(&ts_meta("01AAA")).await.unwrap();
    let patch = SessionPatch {
        title: Some("generated title".into()),
        ..Default::default()
    };
    mirror.update_session("01AAA", &patch).await.unwrap();
    assert_eq!(
        mirror
            .registry
            .get("01AAA")
            .await
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("generated title")
    );

    // A plain session's title patch must not create a registry row.
    let mut plain = ts_meta("02BBB");
    plain.model = Some("m".into());
    mirror.create_session(&plain).await.unwrap();
    mirror.update_session("02BBB", &patch).await.unwrap();
    assert!(mirror.registry.get("02BBB").await.unwrap().is_none());
}

#[tokio::test]
async fn delete_session_unregisters() {
    let (mirror, _tmp, _workdir) = fixture().await;
    mirror.create_session(&ts_meta("01AAA")).await.unwrap();
    mirror.delete_session("01AAA").await.unwrap();

    assert!(mirror.registry.get("01AAA").await.unwrap().is_none());
    assert!(!mirror.known.lock().await.contains("01AAA"));
    // Inner deletion propagated: the store row is gone too.
    assert!(mirror.inner.get_session("01AAA").await.unwrap().is_none());
}

#[tokio::test]
async fn clear_other_sessions_prunes_registry() {
    let (mirror, _tmp, _workdir) = fixture().await;
    mirror.create_session(&ts_meta("01AAA")).await.unwrap();
    mirror.create_session(&ts_meta("02BBB")).await.unwrap();
    mirror.create_session(&ts_meta("03CCC")).await.unwrap();

    let removed = mirror.clear_other_sessions("02BBB").await.unwrap();
    assert_eq!(removed, 2);

    let ids: Vec<String> = mirror
        .registry
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, vec!["02BBB".to_string()]);
    let known: Vec<String> = mirror.known.lock().await.iter().cloned().collect();
    assert_eq!(known, vec!["02BBB".to_string()]);
}

#[tokio::test]
async fn maybe_wrap_gates_on_existing_registry() {
    let inner: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let ts_db = tmp.path().join("ts.db");
    let workdir = tmp.path().join("project");
    tokio::fs::create_dir_all(&workdir).await.unwrap();

    // No registry file: the same inner store is returned untouched.
    let unwrapped = maybe_wrap_at(inner.clone(), &workdir, ts_db.clone()).await;
    assert!(Arc::ptr_eq(&inner, &unwrapped), "no ts.db -> no mirror");

    // Registry present: a mirror wraps and ts sessions flow into it.
    TsRegistry::open(&ts_db).await.unwrap();
    let wrapped = maybe_wrap_at(inner.clone(), &workdir, ts_db).await;
    assert!(
        !Arc::ptr_eq(&wrapped, &inner),
        "ts.db present -> mirror wraps"
    );
    wrapped.create_session(&ts_meta("01AAA")).await.unwrap();
    let registry = TsRegistry::open(tmp.path().join("ts.db")).await.unwrap();
    assert!(registry.get("01AAA").await.unwrap().is_some());
}

/// Compile-time guarantee that the mirror still implements the full `Store`
/// trait surface (delegation must not silently drop a method).
fn _assert_store(_s: &dyn Store) {}
