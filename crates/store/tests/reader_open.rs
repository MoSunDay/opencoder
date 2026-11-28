//! Read-only store opens beside a live writer: the reader takes no
//! maintenance lease, sees the writer's committed sessions through the
//! shared-WAL path and refuses every write path, while the writer keeps
//! writing undisturbed.
use opencoder_database::maintenance_lock::{self, is_writers_active};
use opencoder_store::{SessionFilter, SessionMeta, Store, TursoStore};

fn meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some(id.into()),
        ..Default::default()
    }
}

async fn listed(store: &TursoStore) -> Vec<String> {
    let mut ids: Vec<String> = store
        .list_sessions(&SessionFilter {
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_iter()
        .map(|item| item.id)
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn reader_store_beside_a_live_writer_reads_committed_rows_and_refuses_writes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.db");
    let writer = TursoStore::open(&path).await.unwrap();
    writer.create_session(&meta("LIVE-1")).await.unwrap();
    let canonical = path.canonicalize().unwrap();
    assert!(
        is_writers_active(&maintenance_lock::try_acquire_exclusive(&canonical).unwrap_err()),
        "the writer pool leases the database"
    );

    // Opens beside the live writer: no lease taken, nothing bootstrapped.
    let reader = TursoStore::open_reader(&path).await.unwrap();
    assert_eq!(listed(&reader).await, ["LIVE-1"]);
    assert!(reader.get_session("LIVE-1").await.unwrap().is_some());

    // Store-level writes are refused by the pool before touching the engine.
    let refused = reader.create_session(&meta("REFUSED")).await.unwrap_err();
    assert!(
        refused.to_string().contains("opened read-only"),
        "{refused}"
    );
    // The diagnostics connection of a reader is query-only as well.
    let connection = reader.conn().await.unwrap();
    let refused = connection
        .execute(
            "INSERT INTO sessions(id, created_at, updated_at) VALUES('RAW', 1, 1)",
            (),
        )
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("query_only"),
        "a raw connection of a reader must not write: {refused}"
    );
    assert!(
        is_writers_active(&maintenance_lock::try_acquire_exclusive(&canonical).unwrap_err()),
        "the reader added no lease: the live writer is still the only owner"
    );

    // The live writer is undisturbed and the reader follows its commits.
    writer.create_session(&meta("LIVE-2")).await.unwrap();
    assert_eq!(listed(&reader).await, ["LIVE-1", "LIVE-2"]);
    drop(writer);
    assert!(
        maintenance_lock::try_acquire_exclusive(&canonical).is_ok(),
        "a reader must not keep the writer's lease alive"
    );
    drop(reader);
}

#[tokio::test]
async fn reader_store_never_creates_a_missing_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("absent.db");
    assert!(
        TursoStore::open_reader(&path).await.is_err(),
        "a reader has no business creating a database"
    );
    assert!(!path.exists(), "nothing must be created on disk");
}
