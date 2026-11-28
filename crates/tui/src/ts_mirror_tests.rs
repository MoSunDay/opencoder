//! Tests for `TsMirrorStore` (see `ts_mirror.rs`). Mirrors the `#[path]`
//! convention used by `app_loop_bugfix_tests.rs` / `composer_delete_tests.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use opencoder_core::Message;
use opencoder_store::{SessionMeta, SessionPatch, Store, TsRecord, TsRegistry, TursoStore};

use super::{maybe_wrap_at, TsMirrorStore};

/// A registry file the mirror can wrap: created by a short-lived writer
/// (schema installed, writer dropped) and then opened read-only, exactly the
/// state `maybe_wrap_at` finds on disk.
async fn registry_reader(ts_db: &Path) -> TsRegistry {
    TsRegistry::open(ts_db).await.unwrap();
    TsRegistry::open_reader(ts_db).await.unwrap()
}

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
    let inner: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path().join("project");
    tokio::fs::create_dir_all(&workdir).await.unwrap();
    let ts_db = tmp.path().join("ts.db");
    let registry = registry_reader(&ts_db).await;
    let mirror = TsMirrorStore::new(inner, registry, ts_db, &workdir);
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
    assert_eq!(
        record.workdir.as_deref(),
        Some(workdir.canonicalize().unwrap().as_path())
    );
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
    let inner: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
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

/// Writer discipline: every process may write `ts.db`. A second writable
/// registry handle stays open for the whole test, standing in for a `ts`
/// command or another TUI process. The mirror still wraps, each of its
/// mutations (a short-lived writer that waits inside the transaction retry
/// instead of being refused or deferred) commits a row an independent reader
/// sees while that other writer is still alive, and the other writer keeps
/// writing in turn without anyone being displaced.
#[tokio::test]
async fn mirror_mutations_commit_beside_a_live_writer() {
    let inner: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let ts_db = tmp.path().join("ts.db");
    let workdir = tmp.path().join("project");
    tokio::fs::create_dir_all(&workdir).await.unwrap();

    // Another process's writer is live at TUI startup and stays live for the
    // whole test: the mirror still wraps instead of silently dropping out.
    let other = TsRegistry::open(&ts_db).await.unwrap();
    let wrapped = maybe_wrap_at(inner.clone(), &workdir, ts_db.clone()).await;
    assert!(
        !Arc::ptr_eq(&wrapped, &inner),
        "a live writer must not disable the mirror"
    );
    let registry = TsRegistry::open_reader(&ts_db).await.unwrap();

    wrapped.create_session(&ts_meta("01AAA")).await.unwrap();
    assert_eq!(
        registry
            .get("01AAA")
            .await
            .unwrap()
            .expect("committed while the other writer is alive")
            .title
            .as_deref(),
        Some("seed title"),
        "the mutation was committed by its short-lived writer"
    );

    // The other writer commits a row of its own in turn; an independent
    // reader sees it beside the mirror's row.
    other
        .upsert(&TsRecord {
            id: "02CLI".into(),
            created_at: 1,
            updated_at: 1,
            title: Some("written by the other writer".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut ids: Vec<String> = registry
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["01AAA".to_string(), "02CLI".to_string()]);

    // Later mutations are each a short-lived writer again; the independent
    // reader sees what each committed while the other writer is still alive.
    let patch = SessionPatch {
        title: Some("after startup".into()),
        ..Default::default()
    };
    wrapped.update_session("01AAA", &patch).await.unwrap();
    assert_eq!(
        registry
            .get("01AAA")
            .await
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("after startup")
    );
    wrapped.delete_session("01AAA").await.unwrap();
    assert!(registry.get("01AAA").await.unwrap().is_none());

    // The mirror never displaced the other writer: it still writes afterwards.
    other.delete("02CLI").await.unwrap();
    assert!(registry.get("02CLI").await.unwrap().is_none());
    assert!(registry.list().await.unwrap().is_empty());
}

/// Resume on a compacted session reads only the tail after the soft-deleted
/// head. Inheriting the `Store` trait default would turn that tail load into
/// a full load plus in-memory drain, so one damaged head row would fail the
/// whole resume even though the row is never used. Delegating keeps the native
/// OFFSET path, which never deserializes skipped rows; a full load of the same
/// session stays strict.
#[tokio::test]
async fn load_messages_after_keeps_the_offset_tail_path() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        TursoStore::open(tmp.path().join("legacy-messages.db"))
            .await
            .unwrap(),
    );
    // A `messages` shape without NOT NULL constraints, so a NULL can be
    // planted in the content column.
    let conn = store.conn().await.unwrap();
    conn.execute("DROP TABLE messages", ()).await.unwrap();
    conn.execute(
        "CREATE TABLE messages (\
           seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL, session_id TEXT NOT NULL,\
           role TEXT NOT NULL, agent TEXT, model TEXT, blocks_json TEXT, usage_json TEXT,\
           created_at INTEGER NOT NULL, synthetic INTEGER, display TEXT, provider_state_json TEXT)",
        (),
    )
    .await
    .unwrap();
    store.create_session(&ts_meta("01LEGACY")).await.unwrap();
    // Compacted head (skipped on resume) carries a NULL content column; the
    // tail is healthy. In production a compacted session (`summary_seq > 0`)
    // is what makes Resume read only the tail; here the same store call is
    // driven directly.
    conn.execute(
        "INSERT INTO messages (id, session_id, role, blocks_json, usage_json, created_at)\
         VALUES ('head-corrupt', '01LEGACY', 'user', NULL, '{}', 1)",
        (),
    )
    .await
    .unwrap();
    conn.execute(
        "INSERT INTO messages (id, session_id, role, blocks_json, usage_json, created_at)\
         VALUES ('tail-ok', '01LEGACY', 'user', '[]', '{}', 2)",
        (),
    )
    .await
    .unwrap();

    let ts_db = tmp.path().join("ts.db");
    let registry = registry_reader(&ts_db).await;
    let mirror = TsMirrorStore::new(store.clone(), registry, ts_db, &tmp.path().join("project"));

    // Tail-only load through the mirror: the damaged head is skipped by
    // OFFSET, so only the healthy tail row is decoded and returned.
    let tail = mirror
        .load_messages_after("01LEGACY", 1)
        .await
        .expect("skipped head rows must not be deserialized");
    assert_eq!(tail.len(), 1, "OFFSET 1 keeps only the tail row");
    assert_eq!(tail[0].id, "tail-ok");

    // The same session still fails a full load: no defaulting, no skipping.
    let full = match mirror.load_messages("01LEGACY").await {
        Ok(messages) => panic!("a NULL blocks_json must stay a hard error: {messages:?}"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        full.contains("messages.blocks_json"),
        "error names the offending column: {full}"
    );
    assert!(
        full.contains("message head-corrupt"),
        "error names the damaged row: {full}"
    );
}

/// Compile-time guarantee that the mirror still implements the full `Store`
/// trait surface (delegation must not silently drop a method).
fn _assert_store(_s: &dyn Store) {}

/// Input lifecycle forwarding (consume / unpromote / orphan recovery and
/// read), kept in its own file so this one stays under the test-file size
/// limit; it reuses the fixtures above through `super::*`.
#[path = "ts_mirror_tests/input_lifecycle.rs"]
mod input_lifecycle;
