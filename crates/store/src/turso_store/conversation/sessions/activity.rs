//! Session activity index (`idx_sessions_activity`) repair.
//!
//! `/task` orders sessions by last activity (`sessions.updated_at`, see
//! [`super::ACTIVITY_EXPR`]). Every message append bumps that stamp inside
//! the append transaction, so the activity index is on the hottest write
//! path in the store.
//!
//! Turso 0.8.3 does not maintain indexes atomically across a rolled-back or
//! aborted transaction: index changes from a failed statement can survive
//! the rollback while the table row keeps its old value (its own regression
//! suite documents this — `test_update_multiple_unique_columns_partial_rollback`).
//! Once `idx_sessions_activity` holds a newer stamp than its table row, every
//! later `UPDATE sessions SET updated_at = ...` seeks an index entry that no
//! longer exists and fails with `SQLITE_CORRUPT` ("IdxDelete: no matching
//! index entry found"), permanently blocking message persistence for that
//! session.
//!
//! The index is derived data, so the repair is always safe: drop it, rebuild
//! it from table data, and retry the write once. [`repair_and_retry`] wraps a
//! transaction body with exactly that behaviour.

use anyhow::{Context, Result};
use std::future::Future;
use turso::Connection;

/// Must stay identical to the post-migration `CREATE_INDEX_SESSION_ACTIVITY`.
pub(super) const CREATE_ACTIVITY_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_sessions_activity ON sessions(updated_at DESC, id DESC)";

/// Whether `error` is the missing-index-entry corruption repaired here.
///
/// Scoped to `IdxDelete` on purpose: other `SQLITE_CORRUPT` shapes (page
/// damage, malformed records) are not repaired by rebuilding this index and
/// must surface unchanged.
pub(super) fn is_missing_index_entry(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<turso::Error>(),
            Some(turso::Error::Corrupt(message)) if message.contains("IdxDelete")
        )
    })
}

/// Drop and recreate the activity index from table data.
pub(super) async fn rebuild_activity_index(conn: &Connection) -> Result<()> {
    super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        conn.execute("DROP INDEX IF EXISTS idx_sessions_activity", ())
            .await?;
        conn.execute(CREATE_ACTIVITY_INDEX, ()).await?;
        Ok(())
    })
    .await
    .context("rebuild session activity index")
}

/// Run the transaction body `work`; on missing-index-entry corruption rebuild
/// the activity index and run it once more.
///
/// `work` must be an idempotent transaction body: the failed attempt has
/// already rolled back before the retry starts, so the retry sees the
/// original database state.
pub(crate) async fn repair_and_retry<F, Fut, T>(conn: &Connection, work: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    match work().await {
        Err(error) if is_missing_index_entry(&error) => {
            tracing::warn!(
                error = %error,
                "session activity index is out of sync with the sessions table; rebuilding"
            );
            rebuild_activity_index(conn).await?;
            work().await
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corrupt() -> anyhow::Error {
        anyhow::Error::new(turso::Error::Corrupt(
            "IdxDelete: no matching index entry found for key [Value(Numeric(Integer(1)))] while seeking"
                .into(),
        ))
        .context("touch session activity in tx")
    }

    #[test]
    fn detects_idx_delete_corruption_through_context() {
        assert!(is_missing_index_entry(&corrupt()));
        assert!(!is_missing_index_entry(&anyhow::anyhow!("plain error")));
        assert!(!is_missing_index_entry(&anyhow::Error::new(
            turso::Error::Corrupt("database disk image is malformed".into())
        )));
        assert!(!is_missing_index_entry(&anyhow::Error::new(
            turso::Error::Constraint("UNIQUE constraint failed: sessions.id".into())
        )));
    }

    #[tokio::test]
    async fn repair_and_retry_rebuilds_then_retries_once() {
        let db = turso::Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE INDEX idx_sessions_activity ON sessions(updated_at DESC, id DESC);
             INSERT INTO sessions VALUES('s1', 1, 1);",
        )
        .await
        .unwrap();

        let attempts = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let value = repair_and_retry(&conn, || {
            let attempts = attempts.clone();
            async move {
                attempts.set(attempts.get() + 1);
                if attempts.get() == 1 {
                    // The retry only happens when the failure is the specific
                    // missing-index-entry corruption.
                    return Err(corrupt());
                }
                Ok(42)
            }
        })
        .await
        .unwrap();

        assert_eq!(value, 42);
        assert_eq!(attempts.get(), 2);
        // The index still exists and is usable after the rebuild.
        let mut rows = conn
            .query(
                "SELECT id FROM sessions INDEXED BY idx_sessions_activity",
                (),
            )
            .await
            .unwrap();
        assert!(rows.next().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn repair_and_retry_surfaces_unrelated_errors_without_retry() {
        let db = turso::Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        let attempts = std::cell::Cell::new(0u32);
        let error = repair_and_retry(&conn, || {
            attempts.set(attempts.get() + 1);
            async move { Err::<(), _>(anyhow::anyhow!("unrelated")) }
        })
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "unrelated");
        assert_eq!(attempts.get(), 1);
    }
}
