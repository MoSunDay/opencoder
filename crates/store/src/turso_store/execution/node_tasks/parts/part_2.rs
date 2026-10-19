/// Flag a pending/running task for cancellation; returns the previous status
/// when the flag landed, else `None` (idempotent and harmless on repeats).
pub async fn request_cancel(conn: &Connection, task_id: &str) -> Result<Option<NodeTaskStatus>> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let Some(current) = current_status(conn, task_id).await? else {
            return Ok(None);
        };
        match current {
            s @ (NodeTaskStatus::Pending | NodeTaskStatus::Running) => {
                conn.execute(
                    "UPDATE node_tasks SET cancel_requested = 1, status = 'cancelling'
                     WHERE id = ?1 AND status = ?2",
                    params![task_id, s.as_str()],
                )
                .await
                .context("request node_task cancel")?;
                Ok(Some(s))
            }
            _ => Ok(None),
        }
    })
    .await
}

pub async fn list_tasks(
    conn: &Connection,
    node_id: &str,
    limit: u32,
) -> Result<Vec<NodeTaskRecord>> {
    let mut stmt = conn
        .prepare_cached(&format!(
            "SELECT {TASK_COLS} FROM node_tasks WHERE node_id = ?1
             ORDER BY created_at DESC LIMIT ?2"
        ))
        .await?;
    let mut rows = stmt.query(params![node_id, i64::from(limit)]).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_task(&r)?);
    }
    Ok(out)
}

/// Fleet-wide listing with optional `node_id` / `status` filters, FIFO order
/// (`created_at ASC, rowid ASC` — same drain order as [`claim_next`], with the
/// same-ms `rowid` tiebreak).
pub async fn list_tasks_filtered(
    conn: &Connection,
    node_id: Option<&str>,
    status: Option<NodeTaskStatus>,
    limit: u32,
) -> Result<Vec<NodeTaskRecord>> {
    let mut where_clauses: Vec<&str> = Vec::new();
    let mut args: Vec<turso::Value> = Vec::new();
    if let Some(nid) = node_id {
        where_clauses.push("node_id = ?");
        args.push(nid.to_string().into());
    }
    if let Some(st) = status {
        where_clauses.push("status = ?");
        args.push(st.as_str().into());
    }
    let mut sql = format!("SELECT {TASK_COLS} FROM node_tasks");
    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY created_at ASC, rowid ASC LIMIT ");
    sql.push_str(&i64::from(limit.max(1)).to_string());
    let mut stmt = conn.prepare_cached(&sql).await?;
    let mut rows = stmt.query(turso::params::Params::Positional(args)).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_task(&r)?);
    }
    Ok(out)
}

/// Reverse lookup by the task's unique synthetic session (`session_id` is
/// `UNIQUE` in the schema, so at most one task can match).
pub async fn get_by_session(conn: &Connection, session_id: &str) -> Result<Option<NodeTaskRecord>> {
    let mut stmt = conn
        .prepare_cached(&format!(
            "SELECT {TASK_COLS} FROM node_tasks WHERE session_id = ?1
             ORDER BY rowid DESC LIMIT 1"
        ))
        .await?;
    let mut rows = stmt.query(params![session_id]).await?;
    match rows.next().await? {
        Some(r) => Ok(Some(row_to_task(&r)?)),
        None => Ok(None),
    }
}

pub async fn get_task(conn: &Connection, task_id: &str) -> Result<Option<NodeTaskRecord>> {
    let mut stmt = conn
        .prepare_cached(&format!(
            "SELECT {TASK_COLS} FROM node_tasks WHERE id = ?1 LIMIT 1"
        ))
        .await?;
    let mut rows = stmt.query(params![task_id]).await?;
    match rows.next().await? {
        Some(r) => Ok(Some(row_to_task(&r)?)),
        None => Ok(None),
    }
}

async fn node_exists(conn: &Connection, node_id: &str) -> Result<bool> {
    let mut stmt = conn
        .prepare_cached("SELECT 1 FROM nodes WHERE id = ?1 LIMIT 1")
        .await?;
    let mut rows = stmt.query(params![node_id]).await?;
    Ok(rows.next().await?.is_some())
}

async fn current_status(conn: &Connection, task_id: &str) -> Result<Option<NodeTaskStatus>> {
    let mut stmt = conn
        .prepare_cached("SELECT status FROM node_tasks WHERE id = ?1 LIMIT 1")
        .await?;
    let mut rows = stmt.query(params![task_id]).await?;
    match rows.next().await? {
        Some(r) => Ok(Some(NodeTaskStatus::parse(&r.get::<String>(0)?))),
        None => Ok(None),
    }
}

fn row_to_task(r: &turso::Row) -> Result<NodeTaskRecord> {
    Ok(NodeTaskRecord {
        id: r.get(0)?,
        node_id: r.get(1)?,
        session_id: r.get(2)?,
        title: r.get(3)?,
        prompt: r.get(4)?,
        agent: r.get(5)?,
        model: r.get(6)?,
        status: NodeTaskStatus::parse(&r.get::<String>(7)?),
        error: r.get(8)?,
        cancel_requested: r.get::<i64>(9)? != 0,
        created_at: r.get(10)?,
        claimed_at: r.get(11)?,
        finished_at: r.get(12)?,
    })
}
