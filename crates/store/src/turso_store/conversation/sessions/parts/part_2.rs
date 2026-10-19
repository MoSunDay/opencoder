/// Delete sessions by id, chunked to keep the IN list bounded. Child rows
/// (messages, inputs, events, subagent_tasks) are removed by their
/// `ON DELETE CASCADE` foreign keys. Unknown ids are ignored. Returns the
/// number of deleted session rows.
pub async fn delete_many(conn: &Connection, ids: &[String]) -> Result<u64> {
    let mut removed = 0u64;
    for chunk in ids.chunks(100) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let affected = conn
            .execute(
                &format!("DELETE FROM sessions WHERE id IN ({placeholders})"),
                params_from_iter(chunk.iter().map(String::as_str)),
            )
            .await
            .context("delete sessions by id")?;
        removed += affected as u64;
    }
    Ok(removed)
}

/// Legacy agent-name normalization, applied on READ paths only: databases
/// written during the sandbox-mode interlude store `agent = 'sandbox'` for
/// the read-only agent, which is named `plan` again. Resume must land those
/// rows on the restored plan agent, so map `'sandbox'` -> `'plan'`. The raw
/// stored value is never rewritten.
fn normalize_agent(agent: Option<String>) -> Option<String> {
    agent.map(|a| {
        if a == "sandbox" {
            "plan".to_string()
        } else {
            a
        }
    })
}

fn row_to_meta(r: &turso::Row) -> Result<SessionMeta> {
    Ok(SessionMeta {
        id: r.get::<String>(0)?,
        kind: r.get::<Option<String>>(1)?,
        title: r.get::<Option<String>>(2)?,
        agent: normalize_agent(r.get::<Option<String>>(3)?),
        model: r.get::<Option<String>>(4)?,
        autopilot_mode: r.get::<Option<String>>(16)?,
        workdir_hash: r.get::<Option<String>>(5)?,
        created_at: r.get::<i64>(6)?,
        updated_at: r.get::<i64>(7)?,
        summary: r.get::<Option<String>>(8)?,
        summary_seq: r.get::<Option<i64>>(9)?,
        summary_images: serde_json::from_str(
            r.get::<Option<String>>(10)?.as_deref().unwrap_or("[]"),
        )
        .unwrap_or_default(),
        handoff_seq: r.get::<Option<i64>>(11)?,
        handoff_plan: r.get::<Option<String>>(12)?,
        skill: r.get::<Option<String>>(13)?,
        task_type: r.get::<Option<String>>(14)?,
        requirement: r.get::<Option<String>>(15)?,
    })
}

/// Sort/pagination key for session listings: the session's last activity
/// timestamp, falling back to `created_at` for rows never touched since
/// insertion (imported backfills carry `updated_at = 0`). Ordering by this —
/// instead of `created_at` — is what makes an actively-used (or just
/// clear-context-ed) session float to the top of `/task`.
const ACTIVITY_EXPR: &str = "MAX(s.updated_at, s.created_at)";

/// Cursor = opaque `{activity_ts}|{id}` (both URL-safe: numeric ts + ULID id).
/// `activity_ts` is the row's `ACTIVITY_EXPR` value at read time; there are no
/// external cursor producers, so the key change is decode-compatible.
fn decode_cursor(c: &str) -> Option<(i64, String)> {
    let mut it = c.splitn(2, '|');
    let ts: i64 = it.next()?.parse().ok()?;
    let id = it.next()?.to_string();
    Some((ts, id))
}

fn extract_preview(blocks_json: &Option<String>) -> String {
    let raw = match blocks_json {
        Some(s) => s,
        None => return String::new(),
    };
    let v: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    if let Some(arr) = v.as_array() {
        for b in arr {
            if b.get("kind").and_then(|k| k.as_str()) == Some("text") {
                if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                    return t.chars().take(80).collect();
                }
            }
        }
    }
    String::new()
}
