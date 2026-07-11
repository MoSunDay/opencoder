use anyhow::{Context, Result};
use libsql::{params, Connection};
use opencode_core::{ContentBlock, Message, MessageUsage, Role};

use crate::types::ImportReport;

const INSERT_MESSAGE: &str = "\
INSERT INTO messages (id, session_id, role, agent, model, blocks_json, usage_json, created_at, synthetic, mode, summary)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, 0)";

pub async fn append(conn: &Connection, session_id: &str, msg: &Message) -> Result<i64> {
    let blocks_json = serde_json::to_string(&msg.blocks).context("serialize blocks")?;
    let usage_json = serde_json::to_string(&msg.usage).context("serialize usage")?;
    let role = role_str(msg.role);
    conn.execute(
        INSERT_MESSAGE,
        params![
            msg.id.as_str(),
            session_id,
            role,
            msg.agent.as_deref(),
            msg.model.as_deref(),
            blocks_json,
            usage_json,
            msg.created_at,
            msg.synthetic as i64,
        ],
    )
    .await
    .context("insert message")?;
    last_seq(conn, session_id).await
}

pub async fn append_many(
    conn: &Connection,
    session_id: &str,
    msgs: &[Message],
) -> Result<Vec<i64>> {
    let tx = conn.transaction().await.context("begin tx")?;
    let mut seqs = Vec::with_capacity(msgs.len());
    for m in msgs {
        let blocks_json = serde_json::to_string(&m.blocks).context("serialize blocks")?;
        let usage_json = serde_json::to_string(&m.usage).context("serialize usage")?;
        tx.execute(
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
        let seq = last_seq_in_tx(&tx, session_id).await?;
        seqs.push(seq);
    }
    tx.commit().await.context("commit append_many")?;
    Ok(seqs)
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

async fn last_seq_in_tx(tx: &libsql::Transaction, session_id: &str) -> Result<i64> {
    let stmt = tx
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
/// JSONL migration and any bulk-load path.
pub async fn import(conn: &Connection, session_id: &str, msgs: &[Message]) -> Result<ImportReport> {
    if msgs.is_empty() {
        return Ok(ImportReport::default());
    }
    let tx = conn.transaction().await?;
    let mut count = 0u32;
    for m in msgs {
        let blocks_json = serde_json::to_string(&m.blocks)?;
        let usage_json = serde_json::to_string(&m.usage)?;
        tx.execute(
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
    tx.commit().await?;
    Ok(ImportReport {
        sessions: 1,
        messages: count,
        skipped: 0,
    })
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
    let blocks: Vec<ContentBlock> = serde_json::from_str(&blocks_json).unwrap_or_default();
    let usage: MessageUsage = serde_json::from_str(&usage_json).unwrap_or_default();
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
