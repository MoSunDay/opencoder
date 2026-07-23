use anyhow::{bail, Context, Result};
use libsql::{params, Connection};
use tracing::warn;

use crate::types::{Delivery, SessionInput};

const INSERT_INPUT: &str = "\
INSERT INTO session_inputs (id, session_id, delivery, prompt, admitted_seq, promoted_seq)
VALUES (?, ?, ?, ?, ?, NULL)";

pub async fn admit(conn: &Connection, input: &SessionInput) -> Result<i64> {
    super::tx::run_tx(conn, "BEGIN", || async move {
        let admitted_seq = next_admitted_seq(conn, &input.session_id).await?;
        conn.execute(
            INSERT_INPUT,
            params![
                input.id.as_str(),
                input.session_id.as_str(),
                input.delivery.as_str(),
                input.prompt.as_str(),
                admitted_seq,
            ],
        )
        .await
        .context("insert input")?;
        last_input_seq_in_tx(conn).await
    })
    .await
}

pub async fn pending(
    conn: &Connection,
    session_id: &str,
    delivery: Delivery,
) -> Result<Vec<SessionInput>> {
    let stmt = conn
        .prepare("SELECT seq, id, session_id, delivery, prompt, admitted_seq, promoted_seq FROM session_inputs WHERE session_id = ? AND delivery = ? AND promoted_seq IS NULL ORDER BY admitted_seq ASC")
        .await?;
    let mut rows = stmt.query(params![session_id, delivery.as_str()]).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_input(&r)?);
    }
    Ok(out)
}

/// Promote all pending inputs of `delivery` whose `admitted_seq <= up_to_admitted_seq`.
/// Returns the seqs of promoted inputs. Idempotent (only touches NULL promoted_seq).
pub async fn promote(
    conn: &Connection,
    session_id: &str,
    up_to_admitted_seq: i64,
    delivery: Delivery,
) -> Result<Vec<i64>> {
    super::tx::run_tx(conn, "BEGIN", || async move {
        let stmt = conn
            .prepare("SELECT seq FROM session_inputs WHERE session_id = ? AND delivery = ? AND promoted_seq IS NULL AND admitted_seq <= ? ORDER BY admitted_seq ASC")
            .await?;
        let mut rows = stmt
            .query(params![session_id, delivery.as_str(), up_to_admitted_seq])
            .await?;
        let mut seqs: Vec<i64> = Vec::new();
        while let Some(r) = rows.next().await? {
            seqs.push(r.get::<i64>(0)?);
        }
        drop(stmt);
        drop(rows);
        let promoted_seq = last_input_seq_in_tx(conn).await? + 1;
        for s in &seqs {
            let n = conn
                .execute(
                    "UPDATE session_inputs SET promoted_seq = ? WHERE seq = ?",
                    params![promoted_seq, s],
                )
                .await?;
            if n == 0 {
                warn!(seq = s, "input vanished during promote");
            }
        }
        Ok(seqs)
    })
    .await
}

/// Promote exactly one (oldest) queued input. Returns its seq, or None if none pending.
pub async fn promote_next_queued(conn: &Connection, session_id: &str) -> Result<Option<i64>> {
    super::tx::run_tx(conn, "BEGIN", || async move {
        let stmt = conn
            .prepare("SELECT seq FROM session_inputs WHERE session_id = ? AND delivery = 'queue' AND promoted_seq IS NULL ORDER BY admitted_seq ASC LIMIT 1")
            .await?;
        let mut rows = stmt.query(params![session_id]).await?;
        let target = match rows.next().await? {
            Some(r) => Some(r.get::<i64>(0)?),
            None => None,
        };
        drop(stmt);
        drop(rows);
        if let Some(s) = target {
            let promoted_seq = last_input_seq_in_tx(conn).await? + 1;
            conn.execute(
                "UPDATE session_inputs SET promoted_seq = ? WHERE seq = ?",
                params![promoted_seq, s],
            )
            .await?;
            Ok(Some(s))
        } else {
            Ok(None)
        }
    })
    .await
}

/// Atomically return the oldest pending queued input WITH its prompt and mark it
/// promoted. The runner drain uses this to consume one queued follow-up at idle.
/// Returns the row seq alongside the input so callers (e.g. the TUI mirror) can
/// reconcile by identity.
pub async fn claim_next_queue(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<(i64, SessionInput)>> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let stmt = conn
            .prepare("SELECT seq, id, session_id, delivery, prompt, admitted_seq, promoted_seq FROM session_inputs WHERE session_id = ? AND delivery = 'queue' AND promoted_seq IS NULL ORDER BY admitted_seq ASC LIMIT 1")
            .await?;
        let mut rows = stmt.query(params![session_id]).await?;
        let claimed = match rows.next().await? {
            Some(r) => {
                let seq: i64 = r.get(0)?;
                let input = row_to_input_full(&r, seq)?;
                Some((seq, input))
            }
            None => None,
        };
        drop(stmt);
        drop(rows);
        if let Some((seq, input)) = claimed {
            let promoted_seq = last_input_seq_in_tx(conn).await? + 1;
            conn.execute(
                "UPDATE session_inputs SET promoted_seq = ? WHERE seq = ?",
                params![promoted_seq, seq],
            )
            .await?;
            Ok(Some((seq, input)))
        } else {
            Ok(None)
        }
    })
    .await
}

/// Delete a pending input by its row seq. Only deletes rows that are still
/// unpromoted (`promoted_seq IS NULL`), so consuming-then-deleting cannot wipe
/// an already-drained audit row. Deleting a missing or already-promoted row
/// matches 0 rows and is not an error (idempotent).
pub async fn delete_input(conn: &Connection, seq: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM session_inputs WHERE seq = ? AND promoted_seq IS NULL",
        params![seq],
    )
    .await?;
    Ok(())
}

/// Swap the drain order of two pending inputs by exchanging their
/// `admitted_seq`. Both rows must belong to `session_id` and be still
/// unpromoted. Used by the TUI queue panel to reorder follow-ups.
pub async fn swap_input_order(
    conn: &Connection,
    session_id: &str,
    seq_a: i64,
    seq_b: i64,
) -> Result<()> {
    if seq_a == seq_b {
        return Ok(());
    }
    super::tx::run_tx(conn, "BEGIN", || async move {
        let stmt = conn
            .prepare("SELECT admitted_seq FROM session_inputs WHERE seq = ? AND session_id = ? AND promoted_seq IS NULL")
            .await?;
        let mut rows = stmt.query(params![seq_a, session_id]).await?;
        let a_val: i64 = match rows.next().await? {
            Some(r) => r.get(0)?,
            None => bail!("input seq {seq_a} not found, not in session, or already promoted"),
        };
        drop(stmt);
        drop(rows);
        let stmt = conn
            .prepare("SELECT admitted_seq FROM session_inputs WHERE seq = ? AND session_id = ? AND promoted_seq IS NULL")
            .await?;
        let mut rows = stmt.query(params![seq_b, session_id]).await?;
        let b_val: i64 = match rows.next().await? {
            Some(r) => r.get(0)?,
            None => bail!("input seq {seq_b} not found, not in session, or already promoted"),
        };
        drop(stmt);
        drop(rows);
        conn.execute(
            "UPDATE session_inputs SET admitted_seq = CASE WHEN seq = ? THEN ? WHEN seq = ? THEN ? END WHERE seq IN (?, ?)",
            params![seq_a, b_val, seq_b, a_val, seq_a, seq_b],
        )
        .await
        .context("swap admitted_seq")?;
        Ok(())
    })
    .await
}

async fn next_admitted_seq(conn: &Connection, session_id: &str) -> Result<i64> {
    let stmt = conn
        .prepare("SELECT COALESCE(MAX(admitted_seq), 0) FROM session_inputs WHERE session_id = ?")
        .await?;
    let mut rows = stmt.query(params![session_id]).await?;
    if let Some(r) = rows.next().await? {
        Ok(r.get::<i64>(0)? + 1)
    } else {
        Ok(1)
    }
}

async fn last_input_seq_in_tx(conn: &Connection) -> Result<i64> {
    let stmt = conn.prepare("SELECT MAX(seq) FROM session_inputs").await?;
    let mut rows = stmt.query(()).await?;
    if let Some(r) = rows.next().await? {
        Ok(r.get::<Option<i64>>(0)?.unwrap_or(0))
    } else {
        Ok(0)
    }
}

fn row_to_input(r: &libsql::Row) -> Result<SessionInput> {
    let delivery_s: String = r.get(3)?;
    Ok(SessionInput {
        seq: Some(r.get(0)?),
        id: r.get(1)?,
        session_id: r.get(2)?,
        delivery: Delivery::parse(&delivery_s).unwrap_or_default(),
        prompt: r.get(4)?,
        admitted_seq: r.get(5)?,
        promoted_seq: r.get::<Option<i64>>(6)?,
    })
}

/// Row layout for the claim query: seq, id, session_id, delivery, prompt, admitted_seq, promoted_seq.
fn row_to_input_full(r: &libsql::Row, seq: i64) -> Result<SessionInput> {
    let delivery_s: String = r.get(3)?;
    Ok(SessionInput {
        seq: Some(seq),
        id: r.get(1)?,
        session_id: r.get(2)?,
        delivery: Delivery::parse(&delivery_s).unwrap_or_default(),
        prompt: r.get(4)?,
        admitted_seq: r.get(5)?,
        promoted_seq: r.get::<Option<i64>>(6)?,
    })
}
