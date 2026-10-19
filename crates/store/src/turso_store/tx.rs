//! Transactions run within an exclusive write lease on database workers.

use std::future::Future;

use anyhow::{Context, Result};
use turso::Connection;

/// Commit successful operations and roll back errors before releasing the lease.
pub async fn run_tx<F, Fut, T>(conn: &Connection, begin_sql: &str, work: F) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    anyhow::ensure!(conn.is_autocommit()?, "nested storage transaction");
    conn.execute(begin_sql, ())
        .await
        .context("begin transaction")?;
    match work().await {
        Ok(val) => {
            if let Err(error) = conn.execute("COMMIT", ()).await {
                let _ = conn.execute("ROLLBACK", ()).await;
                return Err(error).context("commit transaction");
            }
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
