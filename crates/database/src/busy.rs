//! Cross-process write-lock admission for every database writer.
//!
//! Turso takes the cross-process write lock with a single non-blocking fcntl
//! try, so `busy_timeout` never converts a contended `BEGIN IMMEDIATE` into
//! waiting — the collision surfaces as `Busy`/`BusySnapshot`. All write paths
//! funnel through this module so contended writers queue with bounded backoff
//! instead of failing, while in-process writers stay serialized by the pool's
//! single write lane.

use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result};
use turso::transaction::{Transaction, TransactionBehavior};
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
pub fn is_busy_conflict(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<turso::Error>()
            .is_some_and(|error| matches!(error, turso::Error::Busy(_) | turso::Error::BusySnapshot(_)))
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

/// Backoff decision for a failed begin: retry only while the failure is a
/// cross-process write-lock conflict AND the connection is still in autocommit
/// (no transaction opened, so every retry is side-effect free). Anything else
/// — a different error, an unknown transaction state, or budget exhaustion —
/// surfaces immediately.
fn retry_delay(
    conn: &Connection,
    error: &anyhow::Error,
    attempt: u32,
    elapsed_ms: u64,
) -> Option<Duration> {
    if conn.is_autocommit().unwrap_or(false) && is_busy_conflict(error) {
        begin_retry_delay(attempt, elapsed_ms, jitter_entropy())
    } else {
        None
    }
}

async fn backoff(attempt: u32, delay: Duration) {
    tracing::warn!(
        attempt = attempt + 1,
        wait_ms = delay.as_millis() as u64,
        "database write lock busy at BEGIN; backing off before retry"
    );
    tokio::time::sleep(delay).await;
}

/// Execute `begin_sql`, retrying cross-process write-lock conflicts with
/// bounded backoff. The caller keeps ownership of COMMIT/ROLLBACK.
pub async fn begin_with_retry(conn: &Connection, begin_sql: &str) -> Result<()> {
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
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let Some(delay) = retry_delay(conn, &error, attempt, elapsed_ms) else {
            return Err(error);
        };
        backoff(attempt, delay).await;
        attempt += 1;
    }
}

/// Begin an immediate transaction, retrying cross-process write-lock
/// conflicts with bounded backoff. `Transaction::new_unchecked` only wraps
/// the connection once the BEGIN succeeded, so a failed begin leaves no
/// transaction and every retry is side-effect free.
pub async fn immediate_with_retry<'conn>(
    conn: &'conn Connection,
) -> Result<Transaction<'conn>> {
    let started = std::time::Instant::now();
    let mut attempt: u32 = 0;
    loop {
        let error = match Transaction::new_unchecked(conn, TransactionBehavior::Immediate).await {
            Ok(transaction) => return Ok(transaction),
            Err(error) => anyhow::Error::new(error).context("begin transaction"),
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let Some(delay) = retry_delay(conn, &error, attempt, elapsed_ms) else {
            return Err(error);
        };
        backoff(attempt, delay).await;
        attempt += 1;
    }
}

/// Begin `begin_sql` with retry, run `work`, then commit on success or roll
/// back on error. A failed rollback is logged and swallowed — never a panic
/// (the pool's incomplete-write guard still rejects a dirty lease).
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
}
