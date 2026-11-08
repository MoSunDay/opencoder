//! Warm-open fast path: a schema-current database opens without taking the
//! cross-process write lock, even while another database handle pins it.

use super::*;

/// A warm open (watermark current, runtime tables present) skips the
/// write-locked bootstrap transaction entirely: while an external connection
/// holds `BEGIN IMMEDIATE`, the open must complete from reads alone instead
/// of queueing on the lock (which would time out here). Writes flow again
/// once the holder commits.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn current_schema_opens_while_an_external_writer_holds_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("warm.db");

    // Bootstrap once, then drop the store so the in-process registry entry
    // dies and the second open builds its own database handle (the shape a
    // second process would have).
    {
        let store = TursoStore::open(&path).await.unwrap();
        store
            .create_session(&SessionMeta {
                id: "warm-1".into(),
                created_at: 1,
                updated_at: 1,
                ..SessionMeta::default()
            })
            .await
            .unwrap();
    }

    let holder = raw_open(&path).await;
    holder
        .busy_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    holder.execute("BEGIN IMMEDIATE", ()).await.unwrap();

    // Without the fast path the open queues on the bootstrap BEGIN IMMEDIATE
    // behind the holder (and the timeout below trips); with it, only reads.
    let store = tokio::time::timeout(std::time::Duration::from_secs(5), TursoStore::open(&path))
        .await
        .expect("warm open must not wait on the cross-process write lock")
        .unwrap();
    assert!(
        store.get_session("warm-1").await.unwrap().is_some(),
        "fast-path store must serve reads"
    );
    let conn = store.conn().await.unwrap();
    assert_eq!(
        count_schema_version_rows(&conn).await,
        1,
        "fast-path open must not touch the version row"
    );

    holder.execute("COMMIT", ()).await.unwrap();
    store
        .create_session(&SessionMeta {
            id: "warm-2".into(),
            created_at: 2,
            updated_at: 2,
            ..SessionMeta::default()
        })
        .await
        .expect("writes must flow again once the holder releases");
}
