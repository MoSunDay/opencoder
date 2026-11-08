//! Transactions run within an exclusive write lease on database workers.

use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result};
use turso::Connection;

/// First backoff after a contended `BEGIN IMMEDIATE`, in milliseconds.
const RETRY_BACKOFF_START_MS: u64 = 25;
/// Backoff ceiling between begin retries, in milliseconds.
const RETRY_BACKOFF_MAX_MS: u64 = 1_000;
/// Total wall-clock budget for begin retries before surfacing the error.
const RETRY_BUDGET_MS: u64 = 120_000;

/// Backoff before retry `attempt` (the count of begins failed so far).
///
/// Exponential growth from [`RETRY_BACKOFF_START_MS`] capped at
/// [`RETRY_BACKOFF_MAX_MS`], plus jitter of up to a quarter of the base
/// derived from `jitter_entropy` (supplied so the schedule stays a pure,
/// unit-pinnable function). `None` once `elapsed_ms` consumed the whole
/// retry budget.
fn begin_retry_delay(attempt: u32, elapsed_ms: u64, jitter_entropy: u64) -> Option<Duration> {
    if elapsed_ms >= RETRY_BUDGET_MS {
        return None;
    }
    let base = RETRY_BACKOFF_START_MS
        .saturating_mul(1_u64 << attempt.min(6))
        .min(RETRY_BACKOFF_MAX_MS);
    let jitter = jitter_entropy % base.div_ceil(4);
    Some(Duration::from_millis(base + jitter))
}

/// Whether the error chain reports a write-lock conflict (`Busy` /
/// `BusySnapshot`). Cross-process lock acquisition is a single non-blocking
/// fcntl try, so this is the only shape a contended `BEGIN IMMEDIATE` takes.
pub(super) fn is_busy_conflict(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<turso::Error>()
            .is_some_and(|error| {
                matches!(
                    error,
                    turso::Error::Busy(_) | turso::Error::BusySnapshot(_)
                )
            })
    })
}

/// Clock-derived entropy for the jitter; hashing through a per-process seed
/// keeps retries from different processes desynchronized.
fn jitter_entropy() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(elapsed.as_secs());
    hasher.write_u64(u64::from(elapsed.subsec_nanos()));
    hasher.finish()
}

/// Execute `begin_sql`, retrying while the failure is a cross-process write
/// lock conflict AND the connection is still in autocommit (no transaction
/// opened, so every retry is side-effect free). Anything else - a different
/// error, an unknown transaction state, or budget exhaustion - surfaces
/// immediately.
async fn begin_with_retry(conn: &Connection, begin_sql: &str) -> Result<()> {
    let started = std::time::Instant::now();
    let mut attempt: u32 = 0;
    loop {
        let failure = match conn.prepare_cached(begin_sql).await {
            Ok(mut statement) => statement.execute(()).await.err(),
            Err(error) => Some(error),
        };
        let Some(error) = failure else {
            return Ok(());
        };
        let error = anyhow::Error::new(error).context("begin transaction");
        let delay = if conn.is_autocommit().unwrap_or(false) && is_busy_conflict(&error) {
            begin_retry_delay(
                attempt,
                started.elapsed().as_millis() as u64,
                jitter_entropy(),
            )
        } else {
            None
        };
        let Some(delay) = delay else {
            return Err(error);
        };
        tracing::warn!(
            attempt = attempt + 1,
            elapsed_ms = started.elapsed().as_millis() as u64,
            wait_ms = delay.as_millis() as u64,
            "database write lock busy at BEGIN; backing off before retry"
        );
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

/// Commit successful operations and roll back errors before releasing the lease.
pub async fn run_tx<F, Fut, T>(conn: &Connection, begin_sql: &str, work: F) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    anyhow::ensure!(conn.is_autocommit()?, "nested storage transaction");
    begin_with_retry(conn, begin_sql).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_retry_delay_grows_exponentially_then_caps() {
        let exact = |attempt: u32| begin_retry_delay(attempt, 0, 0).unwrap().as_millis();
        assert_eq!(exact(0), 25);
        assert_eq!(exact(1), 50);
        assert_eq!(exact(2), 100);
        assert_eq!(exact(3), 200);
        assert_eq!(exact(4), 400);
        assert_eq!(exact(5), 800);
        assert_eq!(exact(6), 1_000, "1600 ms is capped at the ceiling");
        assert_eq!(exact(9), 1_000);
    }

    #[test]
    fn begin_retry_delay_jitter_stays_within_a_quarter_of_base() {
        // attempt 2 has a 100 ms base, so the jitter quarter is 0..=24.
        let with = |jitter: u64| begin_retry_delay(2, 0, jitter).unwrap().as_millis();
        assert_eq!(with(0), 100);
        for jitter in [1u64, 7, 24, 25, 10_000, u64::MAX] {
            assert_eq!(with(jitter), u128::from(100 + jitter % 25));
        }
    }

    #[test]
    fn begin_retry_delay_stops_once_the_budget_is_spent() {
        assert!(begin_retry_delay(0, RETRY_BUDGET_MS, 0).is_none());
        assert!(begin_retry_delay(0, RETRY_BUDGET_MS - 1, 0).is_some());
    }

    #[test]
    fn busy_errors_are_detected_through_context_wrappers() {
        let wrapped = |error: turso::Error| {
            is_busy_conflict(&anyhow::Error::from(error).context("begin transaction"))
        };
        assert!(wrapped(turso::Error::Busy("database is locked".into())));
        assert!(wrapped(turso::Error::BusySnapshot("wal".into())));
        assert!(!wrapped(turso::Error::Constraint("fk".into())));
        assert!(!wrapped(turso::Error::Corrupt("bad page".into())));
    }

    /// Two independent turso databases on one file reproduce the cross-process
    /// write-lock conflict (the OFD locks are per open file description). The
    /// victim's 50 ms busy budget expires long before the holder's 300 ms
    /// lease, so a successful commit is only possible through begin retries.
    #[tokio::test]
    async fn run_tx_survives_cross_connection_write_lock_contention() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("contended.db");
        let path = path.to_str().unwrap().to_owned();

        let connect = |busy: Duration| {
            let path = path.clone();
            async move {
                let database = turso::Builder::new_local(&path)
                    .experimental_multiprocess_wal(true)
                    .build()
                    .await
                    .unwrap();
                let conn = database.connect().unwrap();
                conn.busy_timeout(busy).unwrap();
                conn.execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")
                    .await
                    .unwrap();
                conn
            }
        };

        let holder = connect(Duration::from_secs(30)).await;
        let victim = connect(Duration::from_millis(50)).await;
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

        run_tx(&victim, "BEGIN IMMEDIATE", || async {
            victim.execute("INSERT INTO probe VALUES (1)", ()).await?;
            Ok(())
        })
        .await
        .expect("run_tx must retry past the holder lease and commit");
        lease.await.unwrap();

        let verify = connect(Duration::from_secs(30)).await;
        let mut rows = verify.query("SELECT count(*) FROM probe", ()).await.unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            1,
            "the retried transaction must have committed durably"
        );
    }
}
