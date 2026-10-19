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
        let mut insert = conn.prepare_cached(INSERT_MESSAGE).await?;
        for m in msgs {
            let blocks_json = serde_json::to_string(&m.blocks)?;
            let usage_json = serde_json::to_string(&m.usage)?;
            insert
                .execute(params![
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
                ])
                .await?;
            count += 1;
        }
        Ok(count)
    })
    .await
}

fn row_to_message(r: &turso::Row) -> Result<Message> {
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
