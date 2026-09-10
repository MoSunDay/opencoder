use anyhow::{Context, Result};
use libsql::{Connection, params};

use crate::{TodoEventRecord, TodoItemRecord, TodoWorkflowRecord, TodoWorkflowSummary};

pub async fn create(
    conn: &Connection,
    workflow: &TodoWorkflowRecord,
    items: &[TodoItemRecord],
    event: &TodoEventRecord,
) -> Result<i64> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        insert_workflow(conn, workflow).await?;
        replace_items(conn, items).await?;
        insert_event(conn, event).await
    })
    .await
}

pub async fn commit(
    conn: &Connection,
    workflow: &TodoWorkflowRecord,
    items: &[TodoItemRecord],
    event: &TodoEventRecord,
) -> Result<i64> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        update_workflow(conn, workflow).await?;
        replace_items(conn, items).await?;
        insert_event(conn, event).await
    })
    .await
}

async fn insert_workflow(conn: &Connection, w: &TodoWorkflowRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO todo_workflows (id,parent_session_id,status,spec_json,state_json,generation,created_at,updated_at,terminal_reason) VALUES (?,?,?,?,?,?,?,?,?)",
        params![
            w.id.as_str(), w.parent_session_id.as_str(), w.status.as_str(),
            serde_json::to_string(&w.spec_json)?, serde_json::to_string(&w.state_json)?,
            w.generation, w.created_at, w.updated_at, w.terminal_reason.as_deref()
        ],
    )
    .await
    .context("insert todo workflow")?;
    Ok(())
}

async fn update_workflow(conn: &Connection, w: &TodoWorkflowRecord) -> Result<()> {
    let changed = conn
        .execute(
            "UPDATE todo_workflows SET status=?,state_json=?,generation=?,updated_at=?,terminal_reason=? WHERE id=? AND generation=?",
            params![
                w.status.as_str(), serde_json::to_string(&w.state_json)?, w.generation,
                w.updated_at, w.terminal_reason.as_deref(), w.id.as_str(), w.generation - 1
            ],
        )
        .await
        .context("update todo workflow")?;
    if changed != 1 {
        anyhow::bail!("todo workflow generation conflict: {}", w.id);
    }
    Ok(())
}

async fn replace_items(conn: &Connection, items: &[TodoItemRecord]) -> Result<()> {
    for item in items {
        conn.execute(
            "INSERT INTO todo_items (workflow_id,todo_id,ordinal,status,attempt,active_session_id,session_history_json,result_json,last_error,updated_at) VALUES (?,?,?,?,?,?,?,?,?,?) ON CONFLICT(workflow_id,todo_id) DO UPDATE SET ordinal=excluded.ordinal,status=excluded.status,attempt=excluded.attempt,active_session_id=excluded.active_session_id,session_history_json=excluded.session_history_json,result_json=excluded.result_json,last_error=excluded.last_error,updated_at=excluded.updated_at",
            params![
                item.workflow_id.as_str(), item.todo_id.as_str(), item.ordinal,
                item.status.as_str(), item.attempt, item.active_session_id.as_deref(),
                serde_json::to_string(&item.session_history)?,
                item.result_json.as_ref().map(serde_json::to_string).transpose()?,
                item.last_error.as_deref(), item.updated_at
            ],
        )
        .await
        .context("upsert todo item")?;
    }
    Ok(())
}

async fn insert_event(conn: &Connection, event: &TodoEventRecord) -> Result<i64> {
    conn.execute(
        "INSERT INTO todo_events (workflow_id,kind,payload_json,ts) VALUES (?,?,?,?)",
        params![
            event.workflow_id.as_str(),
            event.kind.as_str(),
            serde_json::to_string(&event.payload)?,
            event.ts
        ],
    )
    .await
    .context("insert todo event")?;
    let mut rows = conn.query("SELECT last_insert_rowid()", ()).await?;
    Ok(rows
        .next()
        .await?
        .map(|row| row.get::<i64>(0))
        .transpose()?
        .unwrap_or(0))
}

pub async fn get(conn: &Connection, id: &str) -> Result<Option<TodoWorkflowRecord>> {
    let mut rows = conn
        .query(
            "SELECT id,parent_session_id,status,spec_json,state_json,generation,created_at,updated_at,terminal_reason FROM todo_workflows WHERE id=?",
            params![id],
        )
        .await?;
    rows.next().await?.map(row_workflow).transpose()
}

pub async fn get_detail(conn: &Connection, id: &str) -> Result<Option<crate::TodoWorkflowDetail>> {
    let mut rows = conn
        .query(
            "SELECT id,parent_session_id,status, \
             CASE WHEN length(CAST(spec_json AS BLOB))<=65536 THEN spec_json END, \
             length(CAST(spec_json AS BLOB)), \
             CASE WHEN length(CAST(state_json AS BLOB))<=65536 THEN state_json END, \
             length(CAST(state_json AS BLOB)),generation,created_at,updated_at,terminal_reason \
             FROM todo_workflows WHERE id=?1",
            params![id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    Ok(Some(crate::TodoWorkflowDetail {
        id: row.get(0)?,
        parent_session_id: row.get(1)?,
        status: row.get(2)?,
        spec_json: bounded_json(row.get(3)?, row.get(4)?, "workflow.spec_json")?,
        state_json: bounded_json(row.get(5)?, row.get(6)?, "workflow.state_json")?,
        generation: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        terminal_reason: row.get(10)?,
    }))
}

fn row_workflow(row: libsql::Row) -> Result<TodoWorkflowRecord> {
    Ok(TodoWorkflowRecord {
        id: row.get(0)?,
        parent_session_id: row.get(1)?,
        status: row.get(2)?,
        spec_json: serde_json::from_str(&row.get::<String>(3)?)?,
        state_json: serde_json::from_str(&row.get::<String>(4)?)?,
        generation: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        terminal_reason: row.get(8)?,
    })
}

pub async fn list(conn: &Connection, limit: u32) -> Result<Vec<TodoWorkflowSummary>> {
    let mut rows = conn
        .query(
            "SELECT id,status,parent_session_id,generation,updated_at FROM todo_workflows ORDER BY updated_at DESC LIMIT ?",
            params![i64::from(limit)],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(TodoWorkflowSummary {
            id: row.get(0)?,
            status: row.get(1)?,
            parent_session_id: row.get(2)?,
            generation: row.get(3)?,
            updated_at: row.get(4)?,
        });
    }
    Ok(out)
}

pub async fn items(conn: &Connection, workflow_id: &str) -> Result<Vec<TodoItemRecord>> {
    let mut rows = conn
        .query(
            "SELECT workflow_id,todo_id,ordinal,status,attempt,active_session_id,session_history_json,result_json,last_error,updated_at FROM todo_items WHERE workflow_id=? ORDER BY ordinal",
            params![workflow_id],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let history: String = row.get(6)?;
        let result: Option<String> = row.get(7)?;
        out.push(TodoItemRecord {
            workflow_id: row.get(0)?,
            todo_id: row.get(1)?,
            ordinal: row.get(2)?,
            status: row.get(3)?,
            attempt: row.get(4)?,
            active_session_id: row.get(5)?,
            session_history: serde_json::from_str(&history)?,
            result_json: result
                .map(|value| serde_json::from_str(&value))
                .transpose()?,
            last_error: row.get(8)?,
            updated_at: row.get(9)?,
        });
    }
    Ok(out)
}

pub async fn items_page(
    conn: &Connection,
    workflow_id: &str,
    after_ordinal: Option<i64>,
    limit: u32,
) -> Result<crate::TodoItemPage> {
    let limit = limit.clamp(1, 100) as usize;
    let mut rows = conn
        .query(
            "SELECT workflow_id,todo_id,ordinal,status,attempt,active_session_id, \
             CASE WHEN length(CAST(session_history_json AS BLOB))<=65536 THEN session_history_json END, \
             length(CAST(session_history_json AS BLOB)), \
             CASE WHEN length(CAST(result_json AS BLOB))<=65536 THEN result_json END, \
             length(CAST(result_json AS BLOB)), \
             CASE WHEN length(CAST(last_error AS BLOB))<=65536 THEN last_error END, \
             length(CAST(last_error AS BLOB)),updated_at FROM todo_items \
             WHERE workflow_id=?1 AND (?2 IS NULL OR ordinal>?2) ORDER BY ordinal LIMIT ?3",
            params![workflow_id, after_ordinal, limit as i64 + 1],
        )
        .await?;
    let mut items = Vec::with_capacity(limit + 1);
    while let Some(row) = rows.next().await? {
        let todo_id: String = row.get(1)?;
        items.push(crate::TodoItemSummary {
            workflow_id: row.get(0)?,
            todo_id: todo_id.clone(),
            ordinal: row.get(2)?,
            status: row.get(3)?,
            attempt: row.get(4)?,
            active_session_id: row.get(5)?,
            session_history: bounded_json(
                row.get(6)?,
                row.get(7)?,
                &format!("todo.item.{todo_id}.session_history"),
            )?,
            result_json: bounded_json(
                row.get(8)?,
                row.get(9)?,
                &format!("todo.item.{todo_id}.result_json"),
            )?,
            last_error: bounded_text(
                row.get(10)?,
                row.get(11)?,
                &format!("todo.item.{todo_id}.last_error"),
            ),
            updated_at: row.get(12)?,
        });
    }
    let more = items.len() > limit;
    items.truncate(limit);
    Ok(crate::TodoItemPage {
        next_ordinal: more.then(|| items.last().unwrap().ordinal),
        items,
    })
}

pub async fn workflow_field_chunk(
    conn: &Connection,
    workflow_id: &str,
    field: &str,
    offset: u64,
    max_bytes: usize,
) -> Result<Option<crate::PayloadChunkRecord>> {
    let column = match field {
        "spec_json" => "spec_json",
        "state_json" => "state_json",
        _ => anyhow::bail!("unsupported todo workflow field"),
    };
    let start = i64::try_from(offset)?.saturating_add(1);
    let take = max_bytes.clamp(1, 64 * 1024) as i64;
    let mut rows = conn
        .query(
            &format!(
                "SELECT length(CAST({column} AS BLOB)), \
                 CAST(substr(CAST({column} AS BLOB),?2,?3) AS BLOB) \
                 FROM todo_workflows WHERE id=?1"
            ),
            params![workflow_id, start, take],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let total = row.get::<i64>(0)?.max(0) as u64;
    if offset > total {
        anyhow::bail!("todo workflow field offset exceeds total bytes");
    }
    Ok(Some(crate::PayloadChunkRecord {
        total_bytes: total,
        bytes: row.get(1)?,
    }))
}

pub async fn item_field_chunk(
    conn: &Connection,
    workflow_id: &str,
    todo_id: &str,
    field: &str,
    offset: u64,
    max_bytes: usize,
) -> Result<Option<crate::PayloadChunkRecord>> {
    let column = match field {
        "session_history" => "session_history_json",
        "result_json" => "result_json",
        "last_error" => "last_error",
        _ => anyhow::bail!("unsupported todo item field"),
    };
    let start = i64::try_from(offset)?.saturating_add(1);
    let take = max_bytes.clamp(1, 64 * 1024) as i64;
    let mut rows = conn
        .query(
            &format!(
                "SELECT length(CAST({column} AS BLOB)), \
                 CAST(substr(CAST({column} AS BLOB),?3,?4) AS BLOB) FROM todo_items \
                 WHERE workflow_id=?1 AND todo_id=?2"
            ),
            params![workflow_id, todo_id, start, take],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let Some(total) = row.get::<Option<i64>>(0)? else {
        return Ok(None);
    };
    let total = total.max(0) as u64;
    if offset > total {
        anyhow::bail!("todo item field offset exceeds total bytes");
    }
    Ok(Some(crate::PayloadChunkRecord {
        total_bytes: total,
        bytes: row.get::<Option<Vec<u8>>>(1)?.unwrap_or_default(),
    }))
}

fn bounded_json(
    value: Option<String>,
    bytes: Option<i64>,
    field: &str,
) -> Result<serde_json::Value> {
    match (value, bytes) {
        (_, None) => Ok(serde_json::Value::Null),
        (Some(value), Some(bytes)) if bytes <= 64 * 1024 => Ok(serde_json::from_str(&value)?),
        (_, Some(bytes)) => Ok(serde_json::json!({
            "omitted": true,
            "total_bytes": bytes.max(0),
            "read_via": "detail_field",
            "field": field,
        })),
    }
}

fn bounded_text(value: Option<String>, bytes: Option<i64>, field: &str) -> serde_json::Value {
    match (value, bytes) {
        (_, None) => serde_json::Value::Null,
        (Some(value), Some(bytes)) if bytes <= 64 * 1024 => serde_json::Value::String(value),
        (_, Some(bytes)) => serde_json::json!({
            "omitted": true,
            "total_bytes": bytes.max(0),
            "read_via": "detail_field",
            "field": field,
        }),
    }
}

pub async fn events_after(
    conn: &Connection,
    workflow_id: &str,
    after: i64,
) -> Result<Vec<TodoEventRecord>> {
    let mut rows = conn
        .query(
            "SELECT seq,workflow_id,kind,payload_json,ts FROM todo_events WHERE workflow_id=? AND seq>? ORDER BY seq",
            params![workflow_id, after],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(TodoEventRecord {
            seq: Some(row.get(0)?),
            workflow_id: row.get(1)?,
            kind: row.get(2)?,
            payload: serde_json::from_str(&row.get::<String>(3)?)?,
            ts: row.get(4)?,
        });
    }
    Ok(out)
}

pub async fn events_page(
    conn: &Connection,
    workflow_id: &str,
    after: i64,
    limit: u32,
    payload_budget: usize,
) -> Result<crate::TodoEventPage> {
    let limit = limit.clamp(1, 200) as usize;
    let mut rows = conn
        .query(
            "SELECT seq,kind,ts,length(CAST(payload_json AS BLOB)) FROM todo_events \
             WHERE workflow_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3",
            params![workflow_id, after, limit as i64 + 1],
        )
        .await?;
    let mut metas = Vec::with_capacity(limit + 1);
    while let Some(row) = rows.next().await? {
        metas.push((
            row.get::<i64>(0)?,
            row.get::<String>(1)?,
            row.get::<i64>(2)?,
            row.get::<i64>(3)?.max(0) as usize,
        ));
    }
    drop(rows);
    let mut events = Vec::with_capacity(limit.min(metas.len()));
    let mut used = 0usize;
    for (seq, kind, ts, payload_len) in metas.iter().take(limit) {
        let bytes = payload_len.saturating_add(512);
        if bytes > payload_budget && events.is_empty() {
            events.push(TodoEventRecord {
                seq: Some(*seq),
                workflow_id: workflow_id.into(),
                kind: kind.clone(),
                payload: serde_json::json!({
                    "omitted": true,
                    "reason": "event_payload_exceeds_page_budget",
                    "total_bytes": payload_len,
                    "read_via": "event_payload",
                }),
                ts: *ts,
            });
            break;
        }
        if used.saturating_add(bytes) > payload_budget {
            break;
        }
        let mut payload = conn
            .query(
                "SELECT payload_json FROM todo_events WHERE workflow_id=?1 AND seq=?2",
                params![workflow_id, *seq],
            )
            .await?;
        let Some(row) = payload.next().await? else {
            anyhow::bail!("todo event {seq} disappeared during pagination");
        };
        events.push(TodoEventRecord {
            seq: Some(*seq),
            workflow_id: workflow_id.into(),
            kind: kind.clone(),
            payload: serde_json::from_str(&row.get::<String>(0)?)?,
            ts: *ts,
        });
        used += bytes;
    }
    Ok(crate::TodoEventPage {
        more: events.len() < metas.len(),
        events,
    })
}

pub async fn event_payload_chunk(
    conn: &Connection,
    workflow_id: &str,
    seq: i64,
    offset: u64,
    max_bytes: usize,
) -> Result<Option<crate::PayloadChunkRecord>> {
    let take = max_bytes.clamp(1, 64 * 1024) as i64;
    let start = i64::try_from(offset)?.saturating_add(1);
    let mut rows = conn
        .query(
            "SELECT length(CAST(payload_json AS BLOB)), \
             CAST(substr(CAST(payload_json AS BLOB),?3,?4) AS BLOB) \
             FROM todo_events WHERE workflow_id=?1 AND seq=?2",
            params![workflow_id, seq, start, take],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let total = row.get::<i64>(0)?.max(0) as u64;
    if offset > total {
        anyhow::bail!("todo event payload offset exceeds total bytes");
    }
    Ok(Some(crate::PayloadChunkRecord {
        total_bytes: total,
        bytes: row.get(1)?,
    }))
}
