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
    conn.prepare_cached(begin_sql)
        .await?
        .execute(())
        .await
        .context("begin transaction")?;
    match work().await {
        Ok(val) => {
            let commit = match conn.prepare_cached("COMMIT").await {
                Ok(mut statement) => statement.execute(()).await,
                Err(error) => Err(error),
            };
            if let Err(error) = commit {
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
