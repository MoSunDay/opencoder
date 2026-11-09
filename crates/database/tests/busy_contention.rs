//! Cross-process write-lock contention exercises the bounded Busy retry:
//! a short-budget victim must queue past a long write lease and commit.
use opencoder_database::busy;
use std::time::Duration;
use turso::Connection;

async fn connect(path: &str, busy: Duration) -> Connection {
    let database = turso::Builder::new_local(path)
        .experimental_multiprocess_wal(true)
        .build()
        .await
        .unwrap();
    let conn = database.connect().unwrap();
    conn.busy_timeout(busy).unwrap();
    conn
}

/// Two independent turso databases on one file reproduce the cross-process
/// write-lock conflict (the OFD locks are per open file description). The
/// victim's 50 ms busy budget expires long before the holder's 300 ms lease,
/// so a successful commit is only possible through begin retries.
#[tokio::test]
async fn run_tx_survives_cross_connection_write_lock_contention() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("contended.db");
    let path = path.to_str().unwrap().to_owned();

    let holder = connect(&path, Duration::from_secs(30)).await;
    let victim = connect(&path, Duration::from_millis(50)).await;
    holder
        .execute("CREATE TABLE probe(id INTEGER)", ())
        .await
        .unwrap();

    holder.execute("BEGIN IMMEDIATE", ()).await.unwrap();
    let lease = {
        let holder = holder;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            holder.execute("COMMIT", ()).await.unwrap();
        })
    };

    busy::run_tx(&victim, "BEGIN IMMEDIATE", || async {
        victim.execute("INSERT INTO probe VALUES (1)", ()).await?;
        Ok(())
    })
    .await
    .expect("run_tx must retry past the holder lease and commit");
    lease.await.unwrap();

    let verify = connect(&path, Duration::from_secs(30)).await;
    let mut rows = verify.query("SELECT count(*) FROM probe", ()).await.unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        1,
        "the retried transaction must have committed durably"
    );
}

/// `immediate_with_retry` must admit a turso `Transaction` past the same
/// cross-process lease so driver-level transactions queue as well.
#[tokio::test]
async fn immediate_with_retry_requeues_past_write_lock_contention() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("contended.db");
    let path = path.to_str().unwrap().to_owned();

    let holder = connect(&path, Duration::from_secs(30)).await;
    let victim = connect(&path, Duration::from_millis(50)).await;
    holder
        .execute("CREATE TABLE probe(id INTEGER)", ())
        .await
        .unwrap();

    holder.execute("BEGIN IMMEDIATE", ()).await.unwrap();
    let lease = {
        let holder = holder;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            holder.execute("COMMIT", ()).await.unwrap();
        })
    };

    let tx = busy::immediate_with_retry(&victim)
        .await
        .expect("the begin must retry past the holder lease");
    tx.execute("INSERT INTO probe VALUES (1)", ()).await.unwrap();
    tx.commit().await.unwrap();
    lease.await.unwrap();

    let verify = connect(&path, Duration::from_secs(30)).await;
    let mut rows = verify.query("SELECT count(*) FROM probe", ()).await.unwrap();
    assert_eq!(rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 1);
}

/// A non-busy begin failure must surface immediately instead of retrying.
#[tokio::test]
async fn immediate_with_retry_surfaces_non_busy_failures() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested.db");
    let path = path.to_str().unwrap().to_owned();
    let conn = connect(&path, Duration::from_secs(30)).await;
    conn.execute("CREATE TABLE probe(id INTEGER)", ()).await.unwrap();

    let first = busy::immediate_with_retry(&conn).await.unwrap();
    let nested = busy::immediate_with_retry(&conn).await;
    assert!(nested.is_err(), "a nested begin must fail, not retry");
    assert!(
        !busy::is_busy_conflict(&nested.unwrap_err()),
        "the nested-begin failure is not a lock conflict"
    );
    drop(first);
}
