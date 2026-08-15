use anyhow::{Context, Result};
use libsql::{params, Connection};

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
