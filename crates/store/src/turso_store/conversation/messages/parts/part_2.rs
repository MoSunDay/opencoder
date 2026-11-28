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

/// Immediate transaction, matching `append_chunk_in_tx`: bulk imports must
/// queue behind cross-process writers instead of failing the lock upgrade.
async fn import_chunk_in_tx(conn: &Connection, session_id: &str, msgs: &[Message]) -> Result<u32> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
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

/// Decode one `messages` row. `usage_json` / `synthetic` are non-identifying
/// metadata, so they are read as nullable: a NULL (e.g. a row written under a
/// table shape that lacked the current NOT NULL constraints) decodes with the
/// same defaults a malformed value uses. That tolerance covers only these two
/// columns; content columns stay strict, and failures carry the session,
/// message and column so a damaged row can be located instead of surfacing as
/// a bare "conversion failure".
fn row_to_message(session_id: &str, r: &turso::Row) -> Result<Message> {
    let id: String = r
        .get(0)
        .with_context(|| format!("decode messages.id for session {session_id}"))?;
    decode_message_columns(session_id, &id, r)
}

fn decode_message_columns(session_id: &str, id: &str, r: &turso::Row) -> Result<Message> {
    let context =
        |column: &str| format!("decode messages.{column} for session {session_id} message {id}");
    let role_s: String = r.get(1).with_context(|| context("role"))?;
    let agent: Option<String> = r.get(2).with_context(|| context("agent"))?;
    let model: Option<String> = r.get(3).with_context(|| context("model"))?;
    let blocks_json: String = r.get(4).with_context(|| context("blocks_json"))?;
    let usage_json: Option<String> = r.get(5).with_context(|| context("usage_json"))?;
    let created_at: i64 = r.get(6).with_context(|| context("created_at"))?;
    let synthetic_i: Option<i64> = r.get(7).with_context(|| context("synthetic"))?;
    let display: Option<String> = r.get(8).with_context(|| context("display"))?;
    let provider_state_json: Option<String> =
        r.get(9).with_context(|| context("provider_state_json"))?;
    let blocks: Vec<ContentBlock> = serde_json::from_str(&blocks_json).unwrap_or_else(|e| {
        tracing::warn!(message_id = %id, error = %e, "failed to deserialize message blocks, using empty");
        Vec::new()
    });
    let usage = match usage_json.as_deref() {
        Some(json) => serde_json::from_str(json).unwrap_or_else(|e| {
            tracing::warn!(message_id = %id, error = %e, "failed to deserialize message usage, using default");
            MessageUsage::default()
        }),
        None => MessageUsage::default(),
    };
    let provider_state = provider_state_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .with_context(|| context("provider_state_json"))?;
    Ok(Message {
        provider_state,
        id: id.to_owned(),
        role: parse_role(&role_s),
        blocks,
        model,
        agent,
        usage,
        created_at,
        synthetic: synthetic_i.unwrap_or(0) != 0,
        display,
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
