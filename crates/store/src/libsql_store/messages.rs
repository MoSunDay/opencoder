use anyhow::{Context, Result};
use libsql::{Connection, params};
use opencoder_core::{ContentBlock, Message, MessageUsage, Role};

use crate::types::{ImportReport, MessageChunkPage, MessageChunkRecord, MessageRow};
use opencoder_core::fleet::MessageCursor;

const INSERT_MESSAGE: &str = "\
INSERT INTO messages (id, session_id, role, agent, model, blocks_json, usage_json, created_at, synthetic, display, provider_state_json, mode, summary)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, 0)";

/// Maximum number of messages inserted per transaction in batch operations.
/// Keeping transactions bounded prevents WAL bloat and reduces lock
/// contention under concurrent access.
const BATCH_CHUNK: usize = 200;

pub async fn append(conn: &Connection, session_id: &str, msg: &Message) -> Result<i64> {
    // Delegate to `append_many` so the INSERT + seq read happen inside a single
    // transaction (same `run_tx` + `last_seq_in_tx` pattern). The autocommit +
    // separate `SELECT MAX(seq)` used previously could race across processes.
    let mut seqs = append_many(conn, session_id, std::slice::from_ref(msg)).await?;
    Ok(seqs.remove(0))
}

/// Append multiple messages in batches of `BATCH_CHUNK` (200) messages per
/// transaction.
///
/// **Non-atomic**: if a batch fails mid-way, earlier batches remain persisted.
/// Callers that need all-or-nothing semantics should check the returned `Err`
/// and compensate (e.g., query `messages_after` to determine how many were
/// persisted, or retry the remainder).
pub async fn append_many(
    conn: &Connection,
    session_id: &str,
    msgs: &[Message],
) -> Result<Vec<i64>> {
    let mut all_seqs = Vec::with_capacity(msgs.len());
    for chunk in msgs.chunks(BATCH_CHUNK) {
        let seqs = append_chunk_in_tx(conn, session_id, chunk).await?;
        all_seqs.extend(seqs);
    }
    Ok(all_seqs)
}

async fn append_chunk_in_tx(
    conn: &Connection,
    session_id: &str,
    msgs: &[Message],
) -> Result<Vec<i64>> {
    super::tx::run_tx(conn, "BEGIN", || async move {
        let mut seqs = Vec::with_capacity(msgs.len());
        for m in msgs {
            let blocks_json = serde_json::to_string(&m.blocks).context("serialize blocks")?;
            let usage_json = serde_json::to_string(&m.usage).context("serialize usage")?;
            conn.execute(
                INSERT_MESSAGE,
                params![
                    m.id.as_str(),
                    session_id,
                    role_str(m.role),
                    m.agent.as_deref(),
                    m.model.as_deref(),
                    blocks_json,
                    usage_json,
                    m.created_at,
                    m.synthetic as i64,
                    m.display.as_deref(),
                    m.provider_state
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()?,
                ],
            )
            .await
            .context("insert message in tx")?;
            let seq = last_seq_in_tx(conn, session_id).await?;
            seqs.push(seq);
        }
        // Activity touch: appending a message IS session activity. Keep
        // `sessions.updated_at` monotonic (`MAX` guards against out-of-order
        // backfills) inside the same tx, so listings ordered by recent
        // activity reflect the last persisted message. `messages::import`
        // deliberately bypasses this — bulk history load is not activity.
        if let Some(last_ts) = msgs.iter().map(|m| m.created_at).max() {
            conn.execute(
                "UPDATE sessions SET updated_at = MAX(updated_at, ?) WHERE id = ?",
                params![last_ts, session_id],
            )
            .await
            .context("touch session activity in tx")?;
        }
        Ok(seqs)
    })
    .await
}

pub async fn load(conn: &Connection, session_id: &str) -> Result<Vec<Message>> {
    let stmt = conn
        .prepare("SELECT id, role, agent, model, blocks_json, usage_json, created_at, synthetic, display, provider_state_json FROM messages WHERE session_id = ? ORDER BY seq ASC")
        .await?;
    let mut rows = stmt.query(params![session_id]).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_message(&r)?);
    }
    Ok(out)
}

/// Load messages for a session skipping the first `skip_count` rows (by `seq`
/// ASC), returning only the tail. Uses `LIMIT -1 OFFSET ?` so SQLite scans but
/// does NOT deserialize the skipped rows' `blocks_json` -- the critical win over
/// a full `load()` for long compacted sessions whose head accumulates thousands
/// of soft-deleted messages. `skip_count <= 0` returns all rows.
pub async fn load_after(
    conn: &Connection,
    session_id: &str,
    skip_count: i64,
) -> Result<Vec<Message>> {
    // Mirror the Store trait default's clamp: a negative offset must never
    // reach SQL OFFSET (behavior is SQLite-version-dependent). `<= 0` returns
    // all rows, matching the trait-default semantics.
    let skip_count = skip_count.max(0);
    let stmt = conn
        .prepare("SELECT id, role, agent, model, blocks_json, usage_json, created_at, synthetic, display, provider_state_json FROM messages WHERE session_id = ? ORDER BY seq ASC LIMIT -1 OFFSET ?")
        .await?;
    let mut rows = stmt.query(params![session_id, skip_count]).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_message(&r)?);
    }
    Ok(out)
}

/// Load raw relay rows ([`MessageRow`]): the true per-session `seq` plus the
/// stored `blocks_json` parsed as a JSON value, in `seq` order. This is the
/// P3 node message relay's read model — it must NOT decode blocks into
/// [`ContentBlock`] because the relay forwards exactly what was stored.
pub async fn load_rows(conn: &Connection, session_id: &str) -> Result<Vec<MessageRow>> {
    let stmt = conn
        .prepare(
            "SELECT seq, role, blocks_json, created_at FROM messages \
             WHERE session_id = ? ORDER BY seq ASC",
        )
        .await?;
    let mut rows = stmt.query(params![session_id]).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        let blocks_json: String = r.get(2)?;
        out.push(MessageRow {
            seq: r.get(0)?,
            role: r.get(1)?,
            blocks: serde_json::from_str(&blocks_json).unwrap_or(serde_json::Value::Null),
            created_at: r.get(3)?,
        });
    }
    Ok(out)
}

/// Read message JSON as SQLite BLOB slices. `substr` is applied by SQLite, so
/// the Rust process never receives the whole legacy row merely to paginate it.
pub async fn load_page(
    conn: &Connection,
    session_id: &str,
    mut cursor: MessageCursor,
    chunk_bytes: usize,
    raw_budget: usize,
) -> Result<MessageChunkPage> {
    if chunk_bytes == 0 || raw_budget < chunk_bytes {
        anyhow::bail!("message page budget must contain at least one non-empty chunk");
    }
    let mut chunks = Vec::with_capacity(raw_budget / chunk_bytes);
    let mut used = 0usize;
    while used < raw_budget {
        let take = chunk_bytes.min(raw_budget - used);
        let Some(chunk) = read_chunk(conn, session_id, cursor, take).await? else {
            return Ok(MessageChunkPage {
                chunks,
                next_cursor: None,
            });
        };
        let next = chunk.offset + chunk.bytes.len() as u64;
        if chunk.bytes.is_empty() && next < chunk.total_bytes {
            anyhow::bail!("message chunk query did not advance");
        }
        cursor = if next < chunk.total_bytes {
            MessageCursor {
                seq: chunk.seq,
                offset: next,
            }
        } else {
            MessageCursor {
                seq: chunk.seq,
                offset: 0,
            }
        };
        used += chunk.bytes.len();
        chunks.push(chunk);
        if used == raw_budget {
            break;
        }
    }
    let next_cursor = has_more(conn, session_id, cursor).await?.then_some(cursor);
    Ok(MessageChunkPage {
        chunks,
        next_cursor,
    })
}

async fn read_chunk(
    conn: &Connection,
    session_id: &str,
    cursor: MessageCursor,
    chunk_bytes: usize,
) -> Result<Option<MessageChunkRecord>> {
    let (sql, seq, offset) = if cursor.offset == 0 {
        (
            "SELECT seq,role,created_at,length(CAST(blocks_json AS BLOB)),\
             CAST(substr(CAST(blocks_json AS BLOB),1,?3) AS BLOB) FROM messages \
             WHERE session_id=?1 AND seq>?2 ORDER BY seq ASC LIMIT 1",
            cursor.seq,
            0,
        )
    } else {
        (
            "SELECT seq,role,created_at,length(CAST(blocks_json AS BLOB)),\
             CAST(substr(CAST(blocks_json AS BLOB),?3+1,?4) AS BLOB) FROM messages \
             WHERE session_id=?1 AND seq=?2 LIMIT 1",
            cursor.seq,
            cursor.offset,
        )
    };
    let mut rows = if offset == 0 {
        conn.query(sql, params![session_id, seq, chunk_bytes as i64])
            .await?
    } else {
        conn.query(
            sql,
            params![session_id, seq, offset as i64, chunk_bytes as i64],
        )
        .await?
    };
    rows.next()
        .await?
        .map(|row| {
            Ok(MessageChunkRecord {
                seq: row.get(0)?,
                role: row.get(1)?,
                created_at: row.get(2)?,
                offset,
                total_bytes: row.get::<i64>(3)?.max(0) as u64,
                bytes: row.get(4)?,
            })
        })
        .transpose()
}

async fn has_more(conn: &Connection, session_id: &str, cursor: MessageCursor) -> Result<bool> {
    if cursor.offset > 0 {
        return Ok(true);
    }
    let mut rows = conn
        .query(
            "SELECT 1 FROM messages WHERE session_id=?1 AND seq>?2 LIMIT 1",
            params![session_id, cursor.seq],
        )
        .await?;
    Ok(rows.next().await?.is_some())
}

pub async fn last_seq(conn: &Connection, session_id: &str) -> Result<i64> {
    let stmt = conn
        .prepare("SELECT MAX(seq) FROM messages WHERE session_id = ?")
        .await?;
    let mut rows = stmt.query(params![session_id]).await?;
    if let Some(r) = rows.next().await? {
        Ok(r.get::<Option<i64>>(0)?.unwrap_or(0))
    } else {
        Ok(0)
    }
}

async fn last_seq_in_tx(conn: &Connection, session_id: &str) -> Result<i64> {
    let stmt = conn
        .prepare("SELECT MAX(seq) FROM messages WHERE session_id = ?")
        .await?;
    let mut rows = stmt.query(params![session_id]).await?;
    if let Some(r) = rows.next().await? {
        Ok(r.get::<Option<i64>>(0)?.unwrap_or(0))
    } else {
        Ok(0)
    }
}

/// Transactional import with count; returns a report. Used by the one-time
/// JSONL migrations and any bulk-load path.
pub async fn import(conn: &Connection, session_id: &str, msgs: &[Message]) -> Result<ImportReport> {
    if msgs.is_empty() {
        return Ok(ImportReport::default());
    }
    let mut count = 0u32;
    for chunk in msgs.chunks(BATCH_CHUNK) {
        count += import_chunk_in_tx(conn, session_id, chunk).await?;
    }
    Ok(ImportReport {
        sessions: 1,
        messages: count,
        skipped: 0,
    })
}

async fn import_chunk_in_tx(conn: &Connection, session_id: &str, msgs: &[Message]) -> Result<u32> {
    super::tx::run_tx(conn, "BEGIN", || async move {
        let mut count = 0u32;
        for m in msgs {
            let blocks_json = serde_json::to_string(&m.blocks)?;
            let usage_json = serde_json::to_string(&m.usage)?;
            conn.execute(
                INSERT_MESSAGE,
                params![
                    m.id.as_str(),
                    session_id,
                    role_str(m.role),
                    m.agent.as_deref(),
                    m.model.as_deref(),
                    blocks_json,
                    usage_json,
                    m.created_at,
                    m.synthetic as i64,
                    m.display.as_deref(),
                    m.provider_state
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()?,
                ],
            )
            .await?;
            count += 1;
        }
        Ok(count)
    })
    .await
}

fn row_to_message(r: &libsql::Row) -> Result<Message> {
    let id: String = r.get(0)?;
    let role_s: String = r.get(1)?;
    let agent: Option<String> = r.get(2)?;
    let model: Option<String> = r.get(3)?;
    let blocks_json: String = r.get(4)?;
    let usage_json: String = r.get(5)?;
    let created_at: i64 = r.get(6)?;
    let synthetic_i: i64 = r.get(7)?;
    let blocks: Vec<ContentBlock> = serde_json::from_str(&blocks_json).unwrap_or_else(|e| {
        tracing::warn!(message_id = %id, error = %e, "failed to deserialize message blocks, using empty");
        Vec::new()
    });
    let usage: MessageUsage = serde_json::from_str(&usage_json).unwrap_or_else(|e| {
        tracing::warn!(message_id = %id, error = %e, "failed to deserialize message usage, using default");
        MessageUsage::default()
    });
    Ok(Message {
        provider_state: r
            .get::<Option<String>>(9)?
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .context("decode provider state")?,
        id,
        role: parse_role(&role_s),
        blocks,
        model,
        agent,
        usage,
        created_at,
        synthetic: synthetic_i != 0,
        display: r.get(8)?,
    })
}

fn role_str(r: Role) -> &'static str {
    match r {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn parse_role(s: &str) -> Role {
    match s {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::User,
    }
}

pub async fn set_usage(
    conn: &Connection,
    session_id: &str,
    message_id: &str,
    usage: &MessageUsage,
) -> Result<()> {
    let count = conn
        .execute(
            "UPDATE messages SET usage_json = ? WHERE session_id = ? AND id = ?",
            params![serde_json::to_string(usage)?, session_id, message_id],
        )
        .await?;
    anyhow::ensure!(
        count == 1,
        "message missing or duplicated while recording harness usage"
    );
    Ok(())
}
