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
//! it from table data, and retry the write once. [`repair_and_retry_or`] wraps
//! a transaction body with exactly that behaviour.

use anyhow::{Context, Result};
use std::future::Future;
use std::time::Duration;
use turso::Connection;

use crate::store::errors::is_unreadable_storage;

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

const ACTIVITY_INDEX: &str = "idx_sessions_activity";
/// Build target of the runtime rebuild (the v41 swap's idea): the replacement
/// is built under this name first, so an unreadable row fails the build -
/// statement-local rollback, nothing in `sqlite_master` - while the index
/// `/task` lists through stays in place.
const REBUILD_INDEX: &str = "idx_sessions_activity_rebuild";

fn rebuild_index_ddl() -> String {
    CREATE_ACTIVITY_INDEX.replace(ACTIVITY_INDEX, REBUILD_INDEX)
}

/// Backoff for the rebuild's full-table scan.
///
/// A failed statement's rollback can leave an all-zero page visible to the
/// next scan for a moment (Turso 0.8.3 releases page pins before restoring
/// pre-images; fixed upstream in 0.9 by invalidating cursors first). The page
/// is not damaged on disk, so a short bounded retry recovers without operator
/// action. A genuinely damaged row stays unreadable and surfaces after the
/// budget is spent.
const REBUILD_BACKOFF: [Duration; 2] = [Duration::from_millis(250), Duration::from_secs(1)];

/// Run `work`, retrying only unreadable-storage failures with `backoff`.
/// Every other error (and unreadable storage once the budget is spent)
/// returns unchanged so callers keep their existing classification.
async fn retry_on_unreadable<F, Fut>(backoff: &[Duration], mut work: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut attempt = 0;
    loop {
        match work().await {
            Ok(()) => return Ok(()),
            Err(error) if is_unreadable_storage(&error) && attempt < backoff.len() => {
                tracing::warn!(
                    attempt = attempt + 1,
                    budget = backoff.len() + 1,
                    error = %error,
                    "unreadable page while building the session activity index; retrying"
                );
                tokio::time::sleep(backoff[attempt]).await;
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Recreate the activity index from table data; probe first, swap second.
///
/// A drop-then-create would leave the index missing when Turso does not roll
/// the DDL back (the desync this module exists for), and the listing requires
/// it by name. The swap therefore only runs once the whole table proved
/// readable; should it still fail, the listing index is restored and the
/// probe the swap never consumed is dropped, both best-effort
/// ([`restore_after_failed_swap`]).
/// Both scans (the probe and the swap's `CREATE INDEX`) get the same bounded
/// retry on unreadable storage: the swap ([`swap_activity_index`]) is
/// idempotent (`DROP IF EXISTS` plus `CREATE`), so a retry after a
/// half-rolled-back attempt converges.
pub(super) async fn rebuild_activity_index(conn: &Connection) -> Result<()> {
    // A probe left by an interrupted swap would skip the readability proof.
    conn.execute(&format!("DROP INDEX IF EXISTS {REBUILD_INDEX}"), ())
        .await?;
    let ddl = rebuild_index_ddl();
    retry_on_unreadable(&REBUILD_BACKOFF, || {
        let ddl = ddl.clone();
        async move {
            super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
                conn.execute(&ddl, ()).await?;
                Ok(())
            })
            .await
        }
    })
    .await
    .context("build session activity index")?;
    let swapped = retry_on_unreadable(&REBUILD_BACKOFF, || swap_activity_index(conn)).await;
    if let Err(error) = swapped {
        restore_after_failed_swap(conn).await;
        return Err(error).context("rebuild session activity index");
    }
    Ok(())
}

/// Best-effort end state after a failed swap: the listing index is back and
/// the probe is gone. The probe was committed by its own transaction, so a
/// failed swap does not take it back; left in place it is maintained by every
/// later session write as a second activity index. Neither statement masks
/// the swap's error, and both are idempotent.
async fn restore_after_failed_swap(conn: &Connection) {
    if let Err(recreate) = conn.execute(CREATE_ACTIVITY_INDEX, ()).await {
        tracing::warn!(error = %recreate, "activity index missing after a failed rebuild");
    }
    if let Err(leftover) = conn
        .execute(&format!("DROP INDEX IF EXISTS {REBUILD_INDEX}"), ())
        .await
    {
        tracing::warn!(
            error = %leftover,
            "activity index probe left behind after a failed rebuild"
        );
    }
}

/// The swap step: drop the stale listing index, create the real one from
/// table data, drop the probe. Every statement is idempotent, so re-running
/// the swap after a half-rolled-back attempt (Turso 0.8.3 does not roll DDL
/// back atomically) converges on `idx_sessions_activity` alone.
async fn swap_activity_index(conn: &Connection) -> Result<()> {
    super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        conn.execute(&format!("DROP INDEX IF EXISTS {ACTIVITY_INDEX}"), ())
            .await?;
        conn.execute(CREATE_ACTIVITY_INDEX, ()).await?;
        conn.execute(&format!("DROP INDEX IF EXISTS {REBUILD_INDEX}"), ())
            .await?;
        Ok(())
    })
    .await
}

/// Run the transaction body `work`; on missing-index-entry corruption rebuild
/// the activity index and run it once more.
///
/// `work` must be an idempotent transaction body: the failed attempt has
/// already rolled back before the retry starts, so the retry sees the
/// original database state. Bodies that can also run without the indexed
/// column provide `degraded`: when the index cannot be rebuilt on unreadable
/// storage (a damaged sessions row), `degraded` — the same write leaving that
/// column untouched — runs instead of failing.
pub(crate) async fn repair_and_retry_or<F, Fut, D, Dfut, T>(
    conn: &Connection,
    work: F,
    degraded: D,
) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
    D: FnOnce() -> Dfut,
    Dfut: Future<Output = Result<T>>,
{
    repair_with(work, degraded, || rebuild_activity_index(conn)).await
}

/// The repair spine with the rebuild injected, so tests can drive its
/// failure modes without a damaged database.
async fn repair_with<F, Fut, D, Dfut, R, Rfut, T>(work: F, degraded: D, rebuild: R) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
    D: FnOnce() -> Dfut,
    Dfut: Future<Output = Result<T>>,
    R: FnOnce() -> Rfut,
    Rfut: Future<Output = Result<()>>,
{
    match work().await {
        Err(error) if is_missing_index_entry(&error) => {
            tracing::warn!(
                error = %error,
                "session activity index is out of sync with the sessions table; rebuilding"
            );
            match rebuild().await {
                Ok(()) => work().await,
                Err(rebuild) if is_unreadable_storage(&rebuild) => {
                    tracing::warn!(
                        error = %rebuild,
                        "session activity index cannot be rebuilt on unreadable storage; \
                         writing without the activity touch"
                    );
                    degraded().await
                }
                Err(rebuild) => Err(rebuild),
            }
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
        let value = repair_and_retry_or(
            &conn,
            || {
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
            },
            || async { Err::<i32, _>(anyhow::anyhow!("degraded body must not run")) },
        )
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
        let error = repair_and_retry_or(
            &conn,
            || {
                attempts.set(attempts.get() + 1);
                async move { Err::<(), _>(anyhow::anyhow!("unrelated")) }
            },
            || async { Err::<(), _>(anyhow::anyhow!("degraded body must not run")) },
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "unrelated");
        assert_eq!(attempts.get(), 1);
    }

    fn idx_delete() -> anyhow::Error {
        anyhow::Error::new(turso::Error::Corrupt(
            "IdxDelete: no matching index entry found for key".into(),
        ))
        .context("touch session activity in tx")
    }

    fn unreadable() -> anyhow::Error {
        anyhow::Error::new(turso::Error::Corrupt(
            "inconsistent overflow chain observed during payload read".into(),
        ))
        .context("rebuild session activity index")
    }

    #[tokio::test]
    async fn degraded_body_runs_when_the_rebuild_hits_unreadable_storage() {
        let work = || async { Err::<(), _>(idx_delete()) };
        let degraded = std::rc::Rc::new(std::cell::Cell::new(false));
        let ran = degraded.clone();
        let out: Result<()> = repair_with(
            work,
            || async move {
                ran.set(true);
                Ok(())
            },
            || async { Err(unreadable()) },
        )
        .await;
        assert!(out.is_ok());
        assert!(degraded.get());
    }

    #[tokio::test]
    async fn other_rebuild_failures_surface_without_running_the_degraded_body() {
        let work = || async { Err::<(), _>(idx_delete()) };
        let degraded = std::cell::Cell::new(false);
        let out: Result<()> = repair_with(
            work,
            || {
                degraded.set(true);
                async { Ok(()) }
            },
            || async { Err(anyhow::anyhow!("database is locked")) },
        )
        .await;
        let error = out.unwrap_err();
        assert!(format!("{error:#}").contains("database is locked"));
        assert!(!degraded.get());
    }

    #[tokio::test]
    async fn successful_rebuild_retries_the_original_body() {
        let attempts = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let work = || {
            let attempts = attempts.clone();
            async move {
                attempts.set(attempts.get() + 1);
                if attempts.get() == 1 {
                    Err(idx_delete())
                } else {
                    Ok(42)
                }
            }
        };
        let degraded = std::cell::Cell::new(false);
        let out = repair_with(
            work,
            || {
                degraded.set(true);
                async { Ok(0) }
            },
            || async { Ok(()) },
        )
        .await;
        assert_eq!(out.unwrap(), 42);
        assert_eq!(attempts.get(), 2);
        assert!(!degraded.get());
    }

    fn unreadable_page() -> anyhow::Error {
        anyhow::Error::new(turso::Error::Corrupt("Invalid page type: 0".into()))
            .context("build session activity index")
    }

    #[tokio::test]
    async fn retry_on_unreadable_recovers_from_a_transient_page() {
        let attempts = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let out = retry_on_unreadable(&[Duration::ZERO, Duration::ZERO], || {
            let attempts = attempts.clone();
            async move {
                attempts.set(attempts.get() + 1);
                if attempts.get() <= 2 {
                    Err(unreadable_page())
                } else {
                    Ok(())
                }
            }
        })
        .await;
        assert!(out.is_ok());
        assert_eq!(attempts.get(), 3);
    }

    #[tokio::test]
    async fn retry_on_unreadable_surfaces_other_errors_immediately() {
        let attempts = std::cell::Cell::new(0u32);
        let error = retry_on_unreadable(&[Duration::ZERO], || {
            attempts.set(attempts.get() + 1);
            async { Err::<(), _>(anyhow::anyhow!("database is locked")) }
        })
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("database is locked"));
        assert_eq!(attempts.get(), 1);
    }

    #[tokio::test]
    async fn retry_on_unreadable_gives_up_after_the_budget() {
        let attempts = std::cell::Cell::new(0u32);
        let error = retry_on_unreadable(&[Duration::ZERO, Duration::ZERO], || {
            attempts.set(attempts.get() + 1);
            async { Err::<(), _>(unreadable_page()) }
        })
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("Invalid page type: 0"));
        assert_eq!(attempts.get(), 3);
    }

    async fn index_names(conn: &Connection) -> Vec<String> {
        let mut rows = conn
            .query(
                "SELECT name FROM sqlite_master WHERE type = 'index' \
                 AND name LIKE 'idx_sessions_activity%' ORDER BY name",
                (),
            )
            .await
            .unwrap();
        let mut names = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            names.push(row.get::<String>(0).unwrap());
        }
        names
    }

    #[tokio::test]
    async fn swap_converges_from_a_half_done_swap() {
        let db = turso::Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE INDEX idx_sessions_activity_rebuild ON sessions(updated_at DESC, id DESC);",
        )
        .await
        .unwrap();
        // The probe finished and the stale listing index is already gone:
        // the state a swap interrupted between its DROP and its CREATE leaves.
        assert_eq!(
            index_names(&conn).await,
            vec!["idx_sessions_activity_rebuild"]
        );
        swap_activity_index(&conn).await.unwrap();
        assert_eq!(index_names(&conn).await, vec!["idx_sessions_activity"]);
        // Re-entering the swap (the retry after an ambiguous failure) is a no-op.
        swap_activity_index(&conn).await.unwrap();
        assert_eq!(index_names(&conn).await, vec!["idx_sessions_activity"]);
    }

    /// A failed swap finds the probe already committed by its own
    /// transaction: the cleanup must drop it beside restoring the listing
    /// index, so later session writes maintain one activity index again.
    #[tokio::test]
    async fn failed_swap_cleanup_drops_the_committed_probe() {
        let db = turso::Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             INSERT INTO sessions VALUES('s1', 1, 100);
             CREATE INDEX idx_sessions_activity_rebuild ON sessions(updated_at DESC, id DESC);",
        )
        .await
        .unwrap();
        // Probe built, listing index dropped: the swap failed in between.
        assert_eq!(
            index_names(&conn).await,
            vec!["idx_sessions_activity_rebuild"]
        );
        restore_after_failed_swap(&conn).await;
        assert_eq!(index_names(&conn).await, vec!["idx_sessions_activity"]);

        // The other half-done shape: the swap failed after recreating the
        // listing index, so only the probe has to go.
        conn.execute_batch(
            "CREATE INDEX idx_sessions_activity_rebuild ON sessions(updated_at DESC, id DESC);
             INSERT INTO sessions VALUES('s2', 2, 200);",
        )
        .await
        .unwrap();
        assert_eq!(
            index_names(&conn).await,
            vec!["idx_sessions_activity", "idx_sessions_activity_rebuild"]
        );
        restore_after_failed_swap(&conn).await;
        assert_eq!(index_names(&conn).await, vec!["idx_sessions_activity"]);

        // The listing index the cleanup put back answers for the rows.
        let mut rows = conn
            .query(
                "SELECT id FROM sessions INDEXED BY idx_sessions_activity ORDER BY updated_at DESC",
                (),
            )
            .await
            .unwrap();
        let mut ids = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            ids.push(row.get::<String>(0).unwrap());
        }
        assert_eq!(ids, vec!["s2", "s1"]);
    }
}
