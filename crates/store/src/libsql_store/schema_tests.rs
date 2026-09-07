//! Unit tests for `schema.rs`, split out to respect the per-file line cap
//! (same pattern as `project/src/service_tests.rs`). `super` still resolves
//! to the schema module, so the imports are unchanged.

use super::{current_version, set_version};
use crate::libsql_store::LibsqlStore;

/// Bug 11: `set_version` wraps DELETE + INSERT in a single transaction so
/// the `schema_version` table is never left empty (nor duplicated) by a
/// crash between the two statements. The replace-not-duplicate invariant
/// is invisible to `current_version` (it uses `LIMIT 1`), so the row count
/// is asserted directly.
#[tokio::test]
async fn set_version_replaces_single_row_atomically() {
    let store = LibsqlStore::open_memory().await.unwrap();
    let conn = store.conn().await.unwrap();

    // `bootstrap` already seeded exactly one row at SCHEMA_VERSION; each
    // subsequent set_version must replace it, never append a second row.
    set_version(&conn, 42).await.unwrap();
    assert_eq!(current_version(&conn).await.unwrap(), Some(42));

    set_version(&conn, 7).await.unwrap();
    assert_eq!(current_version(&conn).await.unwrap(), Some(7));

    // The core regression guard: exactly one row remains. A naive
    // double-INSERT (or a DELETE that outran a crashed INSERT) would leave
    // 2 (or 0) rows here.
    let stmt = conn
        .prepare("SELECT COUNT(*) FROM schema_version")
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let count: i64 = rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap();
    assert_eq!(count, 1, "schema_version must hold exactly one row");
}

/// Regression guard for cold-start fsync storms: `synchronous=NORMAL` must be
/// applied BEFORE `journal_mode=WAL`, because the WAL switch on a fresh
/// database performs header initialization + fsync and honors whatever
/// synchronous policy is active at that moment (default FULL otherwise).
#[test]
fn pragma_order_synchronous_precedes_journal_wal() {
    let idx = |needle: &str| {
        super::PRAGMAS
            .iter()
            .position(|p| p.contains(needle))
            .unwrap_or_else(|| panic!("missing pragma containing {needle}"))
    };
    assert!(idx("busy_timeout") < idx("synchronous"));
    assert!(idx("synchronous") < idx("journal_mode"));
}
