use anyhow::{Context, Result};
use libsql::{params, params_from_iter, Connection, Value};

use crate::types::{SessionFilter, SessionListItem, SessionMeta, SessionPatch};

const INSERT_SESSION: &str = "\
INSERT OR IGNORE INTO sessions (id, title, agent, model, workdir_hash, created_at, updated_at, summary, summary_seq)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)";

pub async fn create(conn: &Connection, meta: &SessionMeta) -> Result<()> {
    conn.execute(
        INSERT_SESSION,
        params![
            meta.id.as_str(),
            meta.title.as_deref(),
            meta.agent.as_deref(),
            meta.model.as_deref(),
            meta.workdir_hash.as_deref(),
            meta.created_at,
            meta.updated_at,
            meta.summary.as_deref(),
            meta.summary_seq,
        ],
    )
    .await
    .context("insert session")?;
    Ok(())
}

pub async fn get(conn: &Connection, id: &str) -> Result<Option<SessionMeta>> {
    let stmt = conn
        .prepare("SELECT id, title, agent, model, workdir_hash, created_at, updated_at, summary, summary_seq FROM sessions WHERE id = ?")
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
        where_clauses.push("(s.id LIKE ? OR COALESCE(s.title,'') LIKE ?)".into());
        let like = format!("%{s}%");
        args.push(like.clone().into());
        args.push(like.into());
    }
    if let Some(cursor) = &filter.cursor {
        if let Some((ts, id)) = decode_cursor(cursor) {
            where_clauses.push("(s.created_at < ? OR (s.created_at = ? AND s.id < ?))".into());
            args.push(ts.into());
            args.push(ts.into());
            args.push(id.into());
        }
    }

    let mut sql = String::from(
        "SELECT s.id, s.title, s.agent, s.model, s.created_at, s.updated_at, \
         (SELECT m.blocks_json FROM messages m WHERE m.session_id = s.id AND m.role = 'user' ORDER BY m.seq ASC LIMIT 1) AS preview \
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
        });
    }
    Ok(out)
}

pub async fn update(conn: &Connection, id: &str, patch: &SessionPatch) -> Result<()> {
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
    if let Some(v) = &patch.summary {
        sets.push("summary = ?");
        args.push(v.clone().into());
    }
    if let Some(v) = patch.summary_seq {
        sets.push("summary_seq = ?");
        args.push(v.into());
    }
    if let Some(v) = patch.updated_at {
        sets.push("updated_at = ?");
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

fn row_to_meta(r: &libsql::Row) -> Result<SessionMeta> {
    Ok(SessionMeta {
        id: r.get::<String>(0)?,
        title: r.get::<Option<String>>(1)?,
        agent: r.get::<Option<String>>(2)?,
        model: r.get::<Option<String>>(3)?,
        workdir_hash: r.get::<Option<String>>(4)?,
        created_at: r.get::<i64>(5)?,
        updated_at: r.get::<i64>(6)?,
        summary: r.get::<Option<String>>(7)?,
        summary_seq: r.get::<Option<i64>>(8)?,
    })
}

/// Cursor = opaque `{created_at}|{id}` (both URL-safe: numeric ts + ULID id).
fn decode_cursor(c: &str) -> Option<(i64, String)> {
    let mut it = c.splitn(2, '|');
    let ts: i64 = it.next()?.parse().ok()?;
    let id = it.next()?.to_string();
    Some((ts, id))
}

#[allow(dead_code)]
pub fn encode_cursor(item: &SessionListItem) -> String {
    format!("{}|{}", item.created_at, item.id)
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
