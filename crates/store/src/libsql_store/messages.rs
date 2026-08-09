use anyhow::{Context, Result};
use libsql::{params, Connection};
use opencoder_core::{ContentBlock, Message, MessageUsage, Role};

use crate::types::ImportReport;

const INSERT_MESSAGE: &str = "\
INSERT INTO messages (id, session_id, role, agent, model, blocks_json, usage_json, created_at, synthetic, mode, summary)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, 0)";

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
                ],
            )
            .await
            .context("insert message in tx")?;
            let seq = last_seq_in_tx(conn, session_id).await?;
            seqs.push(seq);
        }
        Ok(seqs)
    })
    .await
}

pub async fn load(conn: &Connection, session_id: &str) -> Result<Vec<Message>> {
    let stmt = conn
        .prepare("SELECT id, role, agent, model, blocks_json, usage_json, created_at, synthetic FROM messages WHERE session_id = ? ORDER BY seq ASC")
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
        .prepare("SELECT id, role, agent, model, blocks_json, usage_json, created_at, synthetic FROM messages WHERE session_id = ? ORDER BY seq ASC LIMIT -1 OFFSET ?")
        .await?;
    let mut rows = stmt.query(params![session_id, skip_count]).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_message(&r)?);
    }
    Ok(out)
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

async fn import_chunk_in_tx(
    conn: &Connection,
    session_id: &str,
    msgs: &[Message],
) -> Result<u32> {
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
        id,
        role: parse_role(&role_s),
        blocks,
        model,
        agent,
        usage,
        created_at,
        synthetic: synthetic_i != 0,
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
