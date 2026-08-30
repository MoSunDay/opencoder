//! Functional tests for cold-start schema bootstrap behavior on a real
//! on-disk libsql file (WAL semantics exercised truthfully, tempdir-backed).
//!
//! Behavior contracts:
//! - synchronous_is_normal_after_open: the connection ends up with
//!   `synchronous=NORMAL` (1) and `journal_mode=wal`. Order matters: the WAL
//!   switch on a fresh database fsyncs under the *currently active*
//!   synchronous policy, so pragmas must be applied in the safe order or a
//!   cold start pays FULL fsyncs (seconds-minutes inside a ZFS I/O storm).
//! - fresh_open_then_reopen_is_idempotent: fresh open -> one session via the
//!   Store trait -> drop -> two re-opens on the same path. Every re-open
//!   re-runs bootstrap, which must be idempotent: the session survives, the
//!   `schema_version` table still holds exactly one row, and the database
//!   passes `PRAGMA integrity_check`. Running bootstrap inside a single
//!   `BEGIN IMMEDIATE` transaction must not leave nested-transaction debris
//!   (a broken wrapper surfaces as "cannot start a transaction within a
//!   transaction" on the second open).
//! - second_open_on_live_path_succeeds: opening the same path a second time
//!   *while the first store is still alive* — the concurrent-open path whose
//!   `BEGIN IMMEDIATE` queues on the write lock held by busy_timeout.

use opencoder_store::{LibsqlStore, SessionMeta, Store};

/// Read a single-row single-column scalar pragma off a raw connection.
async fn scalar(conn: &libsql::Connection, pragma: &str) -> String {
    let stmt = conn.prepare(pragma).await.unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let row = rows.next().await.unwrap().expect("pragma row");
    row.get::<String>(0).unwrap()
}

async fn count_schema_version_rows(conn: &libsql::Connection) -> i64 {
    let stmt = conn
        .prepare("SELECT COUNT(*) FROM schema_version")
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
}

/// Contract 1: pragma order invariant holds on the *opened* connection, not
/// just in the PRAGMAS const — synchronous must read back NORMAL (1) and the
/// journal must actually be WAL after a cold open.
#[tokio::test]
async fn synchronous_is_normal_after_open() {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("cold.db")).await.unwrap();
    let conn = store.conn().await.unwrap();

    let synchronous: i64 = {
        let stmt = conn.prepare("PRAGMA synchronous").await.unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
    };
    assert_eq!(synchronous, 1, "synchronous must be NORMAL (1) after open");

    assert_eq!(
        scalar(&conn, "PRAGMA journal_mode").await.to_lowercase(),
        "wal",
        "journal_mode must be wal after open"
    );
}

/// Contract 2: bootstrap re-runs (two re-opens on the same path) stay
/// idempotent and leave a healthy, single-versioned database.
#[tokio::test]
async fn fresh_open_then_reopen_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reopen.db");

    // Fresh open + one durable session through the Store trait.
    {
        let store = LibsqlStore::open(&path).await.unwrap();
        store
            .create_session(&SessionMeta {
                id: "boot-s1".into(),
                created_at: 1,
                updated_at: 1,
                ..Default::default()
            })
            .await
            .unwrap();
    }

    // Two re-opens on the same path: each re-runs the (single-transaction)
    // bootstrap. A nested BEGIN would fail the whole open loudly.
    for reopen in 1..=2 {
        let store = LibsqlStore::open(&path).await.unwrap();
        let conn = store.conn().await.unwrap();

        let session = store.get_session("boot-s1").await.unwrap();
        assert!(
            session.is_some(),
            "session must survive re-open #{reopen}"
        );
        assert_eq!(
            count_schema_version_rows(&conn).await,
            1,
            "schema_version must hold exactly one row after re-open #{reopen}"
        );
        assert_eq!(
            scalar(&conn, "PRAGMA integrity_check").await,
            "ok",
            "integrity_check must be ok after re-open #{reopen}"
        );
    }
}

/// Contract 3: concurrent-open path — bootstrap twice against the same file
/// with the first store still alive. The second bootstrap's BEGIN IMMEDIATE
/// must queue on the write lock (busy_timeout) instead of erroring, proving
/// the single-transaction wrapper leaves no transaction state behind.
#[tokio::test]
async fn second_open_on_live_path_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live.db");

    let first = LibsqlStore::open(&path).await.unwrap();
    let second = LibsqlStore::open(&path).await.unwrap();

    assert!(
        first.get_session("none").await.unwrap().is_none(),
        "baseline read on the first store"
    );
    second
        .create_session(&SessionMeta {
            id: "live-s1".into(),
            created_at: 1,
            updated_at: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let seen = first.get_session("live-s1").await.unwrap();
    assert!(seen.is_some(), "both handles must see the same database");

    let conn = first.conn().await.unwrap();
    assert_eq!(
        count_schema_version_rows(&conn).await,
        1,
        "double bootstrap must not duplicate the version row"
    );
}
