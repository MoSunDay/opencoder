use anyhow::{Context, Result};
use libsql::{params, params_from_iter, Connection, Value};

use crate::types::{SessionFilter, SessionListItem, SessionMeta, SessionPatch};

const INSERT_SESSION: &str = "\
INSERT OR IGNORE INTO sessions (id, title, agent, model, autopilot_mode, workdir_hash, created_at, updated_at, summary, summary_seq, summary_images_json, handoff_seq, handoff_plan, skill, task_type, requirement, plan_snapshot, plan_input_count)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

pub async fn create(conn: &Connection, meta: &SessionMeta) -> Result<()> {
    conn.execute(
        INSERT_SESSION,
        params![
            meta.id.as_str(),
            meta.title.as_deref(),
            meta.agent.as_deref(),
            meta.model.as_deref(),
            meta.autopilot_mode.as_deref(),
            meta.workdir_hash.as_deref(),
            meta.created_at,
            meta.updated_at,
            meta.summary.as_deref(),
            meta.summary_seq,
            serde_json::to_string(&meta.summary_images).unwrap_or_else(|_| "[]".into()),
            meta.handoff_seq,
            meta.handoff_plan.as_deref(),
            meta.skill.as_deref(),
            meta.task_type.as_deref().unwrap_or("parent"),
            meta.requirement.as_deref(),
            meta.plan_snapshot.as_deref(),
            meta.plan_input_count,
        ],
    )
    .await
    .context("insert session")?;
    Ok(())
}

pub async fn get(conn: &Connection, id: &str) -> Result<Option<SessionMeta>> {
    let stmt = conn
        .prepare("SELECT id, title, agent, model, workdir_hash, created_at, updated_at, summary, summary_seq, summary_images_json, handoff_seq, handoff_plan, skill, task_type, requirement, plan_snapshot, plan_input_count, autopilot_mode FROM sessions WHERE id = ?")
        .await?;
    let mut rows = stmt.query(params![id]).await?;
    match rows.next().await? {
        Some(r) => Ok(Some(row_to_meta(&r)?)),
        None => Ok(None),
    }
}

pub async fn list(conn: &Connection, filter: &SessionFilter) -> Result<Vec<SessionListItem>> {
    let limit = filter.limit.clamp(1, 500) as i64;
    let mut where_clauses: Vec<String> = Vec::new();
    let mut args: Vec<Value> = Vec::new();

    if let Some(h) = &filter.workdir_hash {
        where_clauses.push("s.workdir_hash = ?".into());
        args.push(h.clone().into());
    }
    if let Some(s) = &filter.search {
        // Escape LIKE metacharacters (`\`, `%`, `_`) in the user-supplied term
        // so they match literally. `ESCAPE '\'` declares the escape character.
        where_clauses
            .push("(s.id LIKE ? ESCAPE '\\' OR COALESCE(s.title,'') LIKE ? ESCAPE '\\')".into());
        let escaped = s
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let like = format!("%{escaped}%");
        args.push(like.clone().into());
        args.push(like.into());
    }
    if let Some(cursor) = &filter.cursor {
        // A malformed cursor is a real error, not a silent fallback to page 1.
        let (ts, id) = decode_cursor(cursor)
            .ok_or_else(|| anyhow::anyhow!("invalid list cursor: {cursor}"))?;
        where_clauses.push("(s.created_at < ? OR (s.created_at = ? AND s.id < ?))".into());
        args.push(ts.into());
        args.push(ts.into());
        args.push(id.into());
    }
    if !filter.include_subagents {
        where_clauses.push("s.task_type = 'parent'".into());
        where_clauses.push(
            "NOT EXISTS (SELECT 1 FROM subagent_tasks st WHERE st.child_session_id = s.id)".into(),
        );
    }

    let mut sql = String::from(
        "SELECT s.id, s.title, s.agent, s.model, s.created_at, s.updated_at, \
         (SELECT substr(m.blocks_json, 1, 8192) FROM messages m WHERE m.session_id = s.id AND m.role = 'user' ORDER BY m.seq ASC LIMIT 1) AS preview, \
         (SELECT COUNT(*) FROM subagent_tasks st WHERE st.parent_session_id = s.id AND st.status = 'running') AS subagent_running, \
         (SELECT COUNT(*) FROM subagent_tasks st WHERE st.parent_session_id = s.id AND st.status = 'cancelled') AS subagent_cancelled, \
         s.skill \
         FROM sessions s",
    );
    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY s.created_at DESC, s.id DESC LIMIT ?");
    args.push(limit.into());

    let stmt = conn.prepare(&sql).await?;
    let mut rows = stmt.query(params_from_iter(args)).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(SessionListItem {
            id: r.get::<String>(0)?,
            title: r.get::<Option<String>>(1)?,
            agent: r.get::<Option<String>>(2)?,
            model: r.get::<Option<String>>(3)?,
            created_at: r.get::<i64>(4)?,
            updated_at: r.get::<i64>(5)?,
            preview: extract_preview(&r.get::<Option<String>>(6)?),
            subagent_running: r.get::<i64>(7)?.max(0) as usize,
            subagent_cancelled: r.get::<i64>(8)?.max(0) as usize,
            skill: r.get::<Option<String>>(9)?,
        });
    }
    Ok(out)
}

pub async fn update(conn: &Connection, id: &str, patch: &SessionPatch) -> Result<()> {
    // Validate: a field value and the clear flag for the same column are
    // contradictory — they would emit both `col = ?` and `col = NULL` SET
    // clauses, producing order-dependent SQL. Reject these combos up front.
    if patch.summary.is_some() && patch.clear_summary {
        anyhow::bail!("SessionPatch: summary field and clear_summary are mutually exclusive");
    }
    if patch.summary_seq.is_some() && patch.clear_summary {
        anyhow::bail!("SessionPatch: summary_seq field and clear_summary are mutually exclusive");
    }
    if patch.summary_images.is_some() && patch.clear_summary {
        anyhow::bail!(
            "SessionPatch: summary_images field and clear_summary are mutually exclusive"
        );
    }
    if patch.handoff_plan.is_some() && patch.clear_handoff {
        anyhow::bail!("SessionPatch: handoff_plan field and clear_handoff are mutually exclusive");
    }
    if patch.handoff_seq.is_some() && patch.clear_handoff {
        anyhow::bail!("SessionPatch: handoff_seq field and clear_handoff are mutually exclusive");
    }
    if patch.skill.is_some() && patch.clear_skill {
        anyhow::bail!("SessionPatch: skill field and clear_skill are mutually exclusive");
    }
    if patch.agent.is_some() && patch.clear_agent {
        anyhow::bail!("SessionPatch: agent field and clear_agent are mutually exclusive");
    }
    if patch.model.is_some() && patch.clear_model {
        anyhow::bail!("SessionPatch: model field and clear_model are mutually exclusive");
    }
    if patch.autopilot_mode.is_some() && patch.clear_autopilot_mode {
        anyhow::bail!(
            "SessionPatch: autopilot_mode field and clear_autopilot_mode are mutually exclusive"
        );
    }
    if patch.requirement.is_some() && patch.clear_requirement {
        anyhow::bail!(
            "SessionPatch: requirement field and clear_requirement are mutually exclusive"
        );
    }
    if patch.plan_snapshot.is_some() && patch.clear_plan_snapshot {
        anyhow::bail!(
            "SessionPatch: plan_snapshot field and clear_plan_snapshot are mutually exclusive"
        );
    }

    let mut sets: Vec<&str> = Vec::new();
    let mut args: Vec<Value> = Vec::new();
    if let Some(v) = &patch.title {
        sets.push("title = ?");
        args.push(v.clone().into());
    }
    if let Some(v) = &patch.agent {
        sets.push("agent = ?");
        args.push(v.clone().into());
    }
    if let Some(v) = &patch.model {
        sets.push("model = ?");
        args.push(v.clone().into());
    }
    if let Some(v) = &patch.autopilot_mode {
        sets.push("autopilot_mode = ?");
        args.push(v.clone().into());
    }
    if let Some(v) = &patch.summary {
        sets.push("summary = ?");
        args.push(v.clone().into());
    }
    if let Some(v) = patch.summary_seq {
        sets.push("summary_seq = ?");
        args.push(v.into());
    }
    if let Some(v) = &patch.summary_images {
        sets.push("summary_images_json = ?");
        args.push(
            serde_json::to_string(v)
                .unwrap_or_else(|_| "[]".into())
                .into(),
        );
    }
    if let Some(v) = patch.handoff_seq {
        sets.push("handoff_seq = ?");
        args.push(v.into());
    }
    if let Some(v) = &patch.handoff_plan {
        sets.push("handoff_plan = ?");
        args.push(v.clone().into());
    }
    if patch.clear_summary {
        sets.push("summary = NULL");
        sets.push("summary_seq = NULL");
        sets.push("summary_images_json = NULL");
    }
    if patch.clear_handoff {
        sets.push("handoff_seq = NULL");
        sets.push("handoff_plan = NULL");
    }
    if let Some(v) = &patch.skill {
        sets.push("skill = ?");
        args.push(v.clone().into());
    }
    if patch.clear_skill {
        sets.push("skill = NULL");
    }
    if patch.clear_agent {
        sets.push("agent = NULL");
    }
    if patch.clear_model {
        sets.push("model = NULL");
    }
    if patch.clear_autopilot_mode {
        sets.push("autopilot_mode = NULL");
    }
    if let Some(v) = patch.updated_at {
        sets.push("updated_at = ?");
        args.push(v.into());
    }
    if let Some(v) = &patch.requirement {
        sets.push("requirement = ?");
        args.push(v.clone().into());
    }
    if patch.clear_requirement {
        sets.push("requirement = NULL");
    }
    if let Some(v) = &patch.plan_snapshot {
        sets.push("plan_snapshot = ?");
        args.push(v.clone().into());
    }
    if patch.clear_plan_snapshot {
        sets.push("plan_snapshot = NULL");
    }
    if let Some(v) = patch.plan_input_count {
        sets.push("plan_input_count = ?");
        args.push(v.into());
    }
    if sets.is_empty() {
        return Ok(());
    }
    let sql = format!("UPDATE sessions SET {} WHERE id = ?", sets.join(", "));
    args.push(id.to_string().into());
    conn.execute(&sql, params_from_iter(args)).await?;
    Ok(())
}

pub async fn delete(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM sessions WHERE id = ?", params![id])
        .await?;
    Ok(())
}

/// Delete all sessions except `keep_id`. Child rows (messages, inputs, events,
/// subagent_tasks) are removed by their `ON DELETE CASCADE` foreign keys.
/// Returns the count of deleted session rows.
pub async fn clear_others(conn: &Connection, keep_id: &str) -> Result<u64> {
    let affected = conn
        .execute("DELETE FROM sessions WHERE id != ?", params![keep_id])
        .await
        .context("clear other sessions")?;
    Ok(affected as u64)
}

fn row_to_meta(r: &libsql::Row) -> Result<SessionMeta> {
    Ok(SessionMeta {
        id: r.get::<String>(0)?,
        title: r.get::<Option<String>>(1)?,
        agent: r.get::<Option<String>>(2)?,
        model: r.get::<Option<String>>(3)?,
        autopilot_mode: r.get::<Option<String>>(17)?,
        workdir_hash: r.get::<Option<String>>(4)?,
        created_at: r.get::<i64>(5)?,
        updated_at: r.get::<i64>(6)?,
        summary: r.get::<Option<String>>(7)?,
        summary_seq: r.get::<Option<i64>>(8)?,
        summary_images: serde_json::from_str(
            r.get::<Option<String>>(9)?.as_deref().unwrap_or("[]"),
        )
        .unwrap_or_default(),
        handoff_seq: r.get::<Option<i64>>(10)?,
        handoff_plan: r.get::<Option<String>>(11)?,
        skill: r.get::<Option<String>>(12)?,
        task_type: r.get::<Option<String>>(13)?,
        requirement: r.get::<Option<String>>(14)?,
        plan_snapshot: r.get::<Option<String>>(15)?,
        plan_input_count: r.get::<Option<i64>>(16)?.unwrap_or(0),
    })
}

/// Cursor = opaque `{created_at}|{id}` (both URL-safe: numeric ts + ULID id).
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
