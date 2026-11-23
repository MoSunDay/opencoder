//! Session activity index (`idx_sessions_activity`) and the `updated_at >=
//! created_at` invariant it relies on.
//!
//! Both the v41 migration and the post-migration index batch go through this
//! module so that a `sessions` table with an unreadable (page-corrupt) row
//! cannot make the database unopenable: Turso fails a whole statement as soon
//! as a scan reaches such a row (`Error::Corrupt`), and the bootstrap
//! transaction is all-or-nothing - a hard failure there would reject every
//! later open, including the one a repair would need. Corrupt-row failures
//! are therefore logged and skipped; the row stays unreadable until the file
//! is dumped and reloaded, everything else keeps working around it.

use anyhow::{Context, Result};
use turso::{params, Connection};

use crate::store::errors::is_corruption_error;

const ACTIVITY_INDEX: &str = "idx_sessions_activity";
/// Build target of the v41 swap. Distinct from [`ACTIVITY_INDEX`] so the
/// legacy index can stay in place until the replacement is known to build.
const PROBE_INDEX: &str = "idx_sessions_activity_v41";

/// The v41 shape, from the post-migration batch definition so the two cannot
/// drift: the batch rebuilds the index under its final name after the swap.
fn probe_index_ddl() -> String {
    super::CREATE_INDEX_SESSION_ACTIVITY.replace(ACTIVITY_INDEX, PROBE_INDEX)
}

/// Whether `sql` (from `sqlite_master`) is the pre-v41 expression index on
/// `MAX(updated_at, created_at)` rather than the plain column index.
fn is_legacy_activity_index(sql: &str) -> bool {
    sql.to_ascii_uppercase()
        .replace(' ', "")
        .contains("MAX(UPDATED_AT,CREATED_AT)")
}

/// v41 migration: swap the legacy expression index for the plain one, then
/// enforce `updated_at >= created_at` so the plain column is the activity
/// stamp the index orders by.
pub(super) async fn migrate(conn: &Connection) -> Result<()> {
    replace_legacy_index(conn).await?;
    backfill_activity_stamp(conn).await
}

/// Post-migration batch: converge `idx_sessions_activity` to the plain shape.
///
/// A database that had to keep its legacy index because of a corrupt row
/// converges here as soon as the row is readable again (dump/reload), so the
/// swap is retried on every open rather than only at the version bump.
pub(super) async fn ensure_index(conn: &Connection) -> Result<()> {
    if stored_index_sql(conn)
        .await?
        .is_some_and(|sql| is_legacy_activity_index(&sql))
    {
        replace_legacy_index(conn).await?;
    }
    // No-op when the index already has its final shape. Only reachable as a
    // real build when no activity index exists at all (legacy index dropped
    // by hand, or the swap just succeeded), so a corrupt table is the only
    // expected failure - and then the legacy index kept above still serves.
    match conn.execute(super::CREATE_INDEX_SESSION_ACTIVITY, ()).await {
        Ok(_) => Ok(()),
        Err(error) if is_corruption_error(&error) => {
            tracing::warn!(
                "sessions table has unreadable rows; leaving idx_sessions_activity as is until the database is repaired"
            );
            Ok(())
        }
        Err(error) => Err(anyhow::Error::new(error)).context("create session activity index"),
    }
}

/// Build the plain index under the probe name first; only when the whole
/// table was readable drop the legacy index (and the probe - the post-
/// migration batch rebuilds the index under its final name). A failed build
/// is rolled back statement-locally by Turso and leaves no trace in
/// `sqlite_master`, so the legacy index is never lost to a corrupt row.
async fn replace_legacy_index(conn: &Connection) -> Result<()> {
    if !probe_build(conn).await? {
        tracing::warn!("keeping the legacy idx_sessions_activity until the database is repaired");
        return Ok(());
    }
    conn.execute(&format!("DROP INDEX IF EXISTS {ACTIVITY_INDEX}"), ())
        .await?;
    conn.execute(&format!("DROP INDEX IF EXISTS {PROBE_INDEX}"), ())
        .await?;
    Ok(())
}

/// Whether every `sessions` row is readable, probed by building the v41 index
/// under the probe name (left in place for the caller to drop).
///
/// This is the only corruption probe that keeps the surrounding bootstrap
/// transaction alive on failure: Turso 0.8.3 ends an explicit transaction as
/// soon as a `SELECT` scan hits an unreadable row, while a failed DDL/DML
/// statement is rolled back statement-locally and the transaction continues.
async fn probe_build(conn: &Connection) -> Result<bool> {
    match conn.execute(probe_index_ddl(), ()).await {
        Ok(_) => Ok(true),
        Err(error) if is_corruption_error(&error) => {
            tracing::warn!(
                error = %error,
                "sessions table has unreadable rows; statements that rewrite it are skipped until the database is repaired"
            );
            Ok(false)
        }
        Err(error) => Err(anyhow::Error::new(error)).context("build session activity index"),
    }
}

/// Set-based fast path; row-by-row fallback that skips unreadable rows.
async fn backfill_activity_stamp(conn: &Connection) -> Result<()> {
    const SET_BASED: &str =
        "UPDATE sessions SET updated_at = created_at WHERE updated_at < created_at";
    match conn.execute(SET_BASED, ()).await {
        Ok(_) => Ok(()),
        Err(error) if is_corruption_error(&error) => {
            tracing::warn!(
                error = %error,
                "sessions table has unreadable rows; backfilling the activity stamp row by row"
            );
            backfill_row_by_row(conn).await
        }
        Err(error) => Err(anyhow::Error::new(error)).context("backfill session activity stamp"),
    }
}

/// Fix every readable row whose stamp predates its creation. Driving the
/// writes from the primary key confines a corrupt row's blast radius to that
/// row: the point UPDATE fails before writing anything, the statement-local
/// rollback keeps the transaction usable, and the remaining rows still
/// converge.
async fn backfill_row_by_row(conn: &Connection) -> Result<()> {
    // Only the covering index scan is safe to walk here (see `probe_build`):
    // it never reads table pages, so it survives rows the table scan cannot.
    // Without a usable index there is nothing better than giving up.
    let mut rows = match conn
        .query(
            &format!("SELECT id FROM sessions INDEXED BY {ACTIVITY_INDEX}"),
            (),
        )
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "activity index unusable; skipping the row-by-row activity stamp backfill"
            );
            return Ok(());
        }
    };
    let mut ids = Vec::new();
    while let Some(row) = rows.next().await? {
        ids.push(row.get::<String>(0)?);
    }
    drop(rows);
    for id in ids {
        let outcome = conn
            .execute(
                "UPDATE sessions SET updated_at = created_at WHERE id = ? AND updated_at < created_at",
                params![id.as_str()],
            )
            .await;
        match outcome {
            Ok(_) => {}
            Err(error) if is_corruption_error(&error) => {
                tracing::warn!(
                    session_id = %id,
                    error = %error,
                    "skipping unreadable session row during activity stamp backfill"
                );
            }
            Err(error) => {
                return Err(anyhow::Error::new(error)).context("backfill session activity stamp")
            }
        }
    }
    Ok(())
}

/// Whether every `sessions` row is readable (see [`probe_build`] for why this
/// is an index build and not a scan). Lets callers skip statements that
/// rewrite the whole table (`ALTER TABLE ... DROP COLUMN`) instead of risking
/// a half-applied rewrite on a damaged table.
pub(super) async fn table_readable(conn: &Connection) -> Result<bool> {
    if !probe_build(conn).await? {
        return Ok(false);
    }
    conn.execute(&format!("DROP INDEX IF EXISTS {PROBE_INDEX}"), ())
        .await?;
    Ok(true)
}

/// Whether `idx_sessions_activity` already has its final (plain) shape. A
/// missing or legacy index converges in the post-migration batch, which a
/// warm open skipping the bootstrap would never run.
pub(super) async fn index_is_current(conn: &Connection) -> Result<bool> {
    Ok(stored_index_sql(conn)
        .await?
        .is_some_and(|sql| !is_legacy_activity_index(&sql)))
}

async fn stored_index_sql(conn: &Connection) -> Result<Option<String>> {
    let mut rows = conn
        .query(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?",
            params![ACTIVITY_INDEX],
        )
        .await?;
    match rows.next().await? {
        Some(row) => Ok(row.get::<Option<String>>(0)?),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::is_legacy_activity_index;

    #[test]
    fn legacy_expression_index_is_recognized_regardless_of_formatting() {
        assert!(is_legacy_activity_index(
            "CREATE INDEX IF NOT EXISTS idx_sessions_activity ON sessions (MAX (updated_at, created_at) DESC, id DESC)"
        ));
        assert!(!is_legacy_activity_index(
            "CREATE INDEX IF NOT EXISTS idx_sessions_activity ON sessions(updated_at DESC, id DESC)"
        ));
    }
}
