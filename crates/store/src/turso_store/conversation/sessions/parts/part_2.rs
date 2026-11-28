/// Delete sessions by id, chunked to keep the IN list bounded. Child rows
/// (messages, inputs, events, subagent_tasks) are removed by their
/// `ON DELETE CASCADE` foreign keys. Unknown ids are ignored. Returns the
/// number of deleted session rows.
pub async fn delete_many(conn: &Connection, ids: &[String]) -> Result<u64> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
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
    })
    .await
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

/// Decode one `sessions` row. `id` / `created_at` / `updated_at` are strict:
/// a NULL or wrongly-typed value is treated as damage and the read fails
/// instead of being defaulted (a fabricated stamp would reorder `/task` and
/// misplace the resume boundary) or skipped (the session would silently
/// vanish). The failure names the session and column so the row can be
/// located; `requested_id` is passed in because the row's own `id` column may
/// be the unreadable one.
fn row_to_meta(requested_id: &str, r: &turso::Row) -> Result<SessionMeta> {
    let id: String = r
        .get(0)
        .with_context(|| format!("decode sessions.id for session {requested_id}"))?;
    let created_at: i64 = r
        .get(6)
        .with_context(|| format!("decode sessions.created_at for session {id}"))?;
    let updated_at: i64 = r
        .get(7)
        .with_context(|| format!("decode sessions.updated_at for session {id}"))?;
    Ok(SessionMeta {
        id,
        kind: r.get::<Option<String>>(1)?,
        title: r.get::<Option<String>>(2)?,
        agent: normalize_agent(r.get::<Option<String>>(3)?),
        model: r.get::<Option<String>>(4)?,
        workdir_hash: r.get::<Option<String>>(5)?,
        created_at,
        updated_at,
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
/// timestamp. `updated_at >= created_at` is a stored invariant (insert clamp
/// and v41 backfill), so the plain column is exactly the activity stamp and
/// `idx_sessions_activity` serves the order directly. Ordering by this —
/// instead of `created_at` — is what makes an actively-used (or just
/// clear-context-ed) session float to the top of `/task`.
const ACTIVITY_EXPR: &str = "s.updated_at";

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
