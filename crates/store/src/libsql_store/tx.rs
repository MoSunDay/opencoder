//! Manual transaction helper that avoids `libsql::Transaction`'s panic-on-drop.
//!
//! `libsql::Transaction::Drop` (0.9.30) calls `do_rollback().unwrap()` which
//! panics when the rollback fails. With a single shared connection serialized
//! by `db_lock`, async cancellation (e.g. `tokio::select!`) can drop the
//! `MutexGuard` before the `Transaction`, letting another task mutate the shared
//! handle and invalidate the transaction state — producing
//! `SqliteFailure(1, "cannot rollback - no transaction is active")`.
//!
//! This helper uses manual `BEGIN`/`COMMIT`/`ROLLBACK` so rollback failures
//! degrade to a logged warning instead of a panic. A best-effort `ROLLBACK`
//! before `BEGIN` recovers from transactions left dangling by future
//! cancellation.

use std::future::Future;

use anyhow::{Context, Result};
use libsql::Connection;

/// Run `work` inside a manual transaction.
///
/// Begins with `begin_sql` (use `"BEGIN"` for deferred or `"BEGIN IMMEDIATE"`
/// for an immediate write lock). Commits on `Ok`, rolls back on `Err`.
/// Rollback failures are logged and swallowed — never panicked.
///
/// The `work` closure takes no arguments; it captures `&Connection` (which is
/// `Copy`) and any other borrowed inputs from the calling scope, and returns
/// `Result<T>`.
pub async fn run_tx<F, Fut, T>(conn: &Connection, begin_sql: &str, work: F) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    // Best-effort cleanup: if a previous transaction was left dangling by
    // future cancellation (the guard dropped after BEGIN but before COMMIT),
    // roll it back now. ROLLBACK with no active transaction is an ignorable
    // error; with one, it restores autocommit mode so our BEGIN succeeds.
    let _ = conn.execute("ROLLBACK", ()).await;
    conn.execute(begin_sql, ())
        .await
        .context("begin transaction")?;
    match work().await {
        Ok(val) => {
            conn.execute("COMMIT", ())
                .await
                .context("commit transaction")?;
            Ok(val)
        }
        Err(e) => {
            if let Err(rb) = conn.execute("ROLLBACK", ()).await {
                tracing::warn!(error = %rb, "transaction rollback failed (swallowed)");
            }
            Err(e)
        }
    }
}
